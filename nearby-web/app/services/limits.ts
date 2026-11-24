/**
 * Oeca Nearby: the bounds on what a peer can make this page hold or do
 * (the same rules as Oeca's src/nearby.rs where both have them).
 */
import pako from "pako";
import { Failure } from "./proof";

/** An SDP, inflated. */
export const MAX_SDP_BYTES = 64 * 1024;
/** An SDP as signaled (compressed, base64): more cannot inflate to a valid one. */
export const MAX_SIGNALED_CHARS = 128 * 1024;
/** One signaling message, before it is parsed. */
export const MAX_SIGNALING_MESSAGE = 1024 * 1024;
/** Echoed into our answer, so a peer does not choose how much we send. */
export const MAX_SESSION_ID = 128;

/** One control message (the file list, the token list), while it arrives. */
export const MAX_CONTROL_BYTES = 1024 * 1024;
export const MAX_CONTROL_CHUNKS = 4096;
/** Queued and unread: until this user accepted, and while files arrive. */
export const CONTROL_BACKLOG = 2 * 1024 * 1024;
// ponytail: RTCDataChannel cannot be paused, so a sender far faster than this
// device's storage fails here; needs app-level flow control if real phones hit it
export const TRANSFER_BACKLOG = 64 * 1024 * 1024;
/** What a queued message costs besides its bytes, so empty ones are not free. */
export const MESSAGE_COST = 256;
/** A file token, as a receiver gives it. */
export const MAX_TOKEN_CHARS = 256;
/** All channels a peer may open in one session: "data" and the extension's are two. */
export const MAX_CHANNELS = 4;

const MAX_LISTED = 1000;
const MAX_NAME_BYTES = 1024;
const MAX_ID_CHARS = 128;

/** Busy: connections made only to decline, at once. */
export const MAX_DECLINES = 2;
/** Busy: answers per window. Each one costs the room's shared signaling quota. */
export const MAX_REFUSALS = 30;
export const MAX_REFUSALS_PER_PEER = 5;
export const REFUSAL_WINDOW_MS = 10 * 60 * 1000;

/** The zlib stream, inflated no further than MAX_SDP_BYTES. */
export function inflateSdp(compressed: Uint8Array): string {
  const inflator = new pako.Inflate({ chunkSize: 16 * 1024 });
  const parts: Uint8Array[] = [];
  let size = 0;
  inflator.onData = (chunk) => {
    const part = chunk as Uint8Array;
    size += part.byteLength;
    // Thrown out of push: nothing more is inflated
    if (size > MAX_SDP_BYTES) throw new Error("SDP too large");
    parts.push(part);
  };
  let complete = false;
  inflator.onEnd = (status) => {
    complete = (status as number) === 0;
  };
  inflator.push(compressed, true);
  if (!complete || inflator.err) throw new Error("Decompression failed.");
  const all = new Uint8Array(size);
  let at = 0;
  for (const part of parts) {
    all.set(part, at);
    at += part.byteLength;
  }
  return new TextDecoder("utf-8", { fatal: true }).decode(all);
}

export type ListedFile = { id: string; fileName: string; size: number; fileType: string };

/** What the sender listed, as exactly that shape, or a refusal. */
export function parseFileList(value: unknown): ListedFile[] {
  const refuse = (why: string) => new Failure("other", `The file list was refused: ${why}.`);
  if (!Array.isArray(value) || value.length === 0) throw refuse("it has no files");
  if (value.length > MAX_LISTED) throw refuse(`more than ${MAX_LISTED} files`);
  const utf8 = new TextEncoder();
  const ids = new Set<string>();
  let total = 0;
  return value.map((entry) => {
    const { id, fileName, size, fileType } = Object(entry);
    if (typeof id !== "string" || !id || id.length > MAX_ID_CHARS) throw refuse("a file id is invalid");
    if (ids.has(id)) throw refuse("two files have the same id");
    ids.add(id);
    if (typeof fileName !== "string" || fileName.length > MAX_NAME_BYTES || utf8.encode(fileName).length > MAX_NAME_BYTES) {
      throw refuse("a file name is invalid");
    }
    if (!Number.isSafeInteger(size) || size < 0) throw refuse("a file size is invalid");
    total += size;
    if (!Number.isSafeInteger(total)) throw refuse("it is too large");
    // oeca: shown and saved as the same safe name (safeName)
    return { id, fileName: safeName(fileName), size, fileType: typeof fileType === "string" ? fileType.slice(0, 255) : "" };
  });
}

/** One approval is each approved file once, with the token given for it. */
export class Approved<T extends { id: string }> {
  #left = new Map<string, { file: T; token: string }>();

  /** Approved and not sent yet. */
  get remaining() {
    return this.#left.size;
  }

  add(file: T, token: string) {
    this.#left.set(file.id, { file, token });
  }

  take(id: unknown, token: unknown): T {
    const entry = typeof id === "string" ? this.#left.get(id) : undefined;
    if (!entry || entry.token !== token) {
      throw new Failure("other", "The sender sent a file that was not approved, or sent one twice.");
    }
    this.#left.delete(entry.file.id);
    return entry.file;
  }
}

/** The answers given to peers that cannot be served now. Past the limit: silence. */
export class Refusals {
  #recent: { at: number; peer: string }[] = [];

  allow(peer: string, now = performance.now()): boolean {
    this.#recent = this.#recent.filter((r) => now - r.at < REFUSAL_WINDOW_MS);
    const byPeer = this.#recent.filter((r) => r.peer === peer).length;
    if (this.#recent.length >= MAX_REFUSALS || byPeer >= MAX_REFUSALS_PER_PEER) return false;
    this.#recent.push({ at: now, peer });
    return true;
  }
}

/** The server's 429: nothing shortens this wait. */
export const RATE_LIMITED_WAIT_MS = 15 * 60 * 1000;
/** A connection that lasted this long resets the backoff. */
export const STABLE_MS = 60_000;
/** Between two connection attempts, whatever asks for the next one. */
export const MIN_RECONNECT_GAP_MS = 10_000;

/**
 * How long until the next connection attempt. `hurried`: the network or the
 * tab came back. `limitedUntil`: the server said 429.
 */
export function reconnectIn(
  at: { now: number; started: number; delay: number; hurried: boolean; limitedUntil: number },
  random = Math.random(),
): number {
  const wait = at.hurried
    ? at.started + MIN_RECONNECT_GAP_MS - at.now
    : at.delay + (at.delay / 4) * random; // jitter: many tabs behind one IP
  return Math.max(0, wait, at.limitedUntil - at.now);
}

// ---- oeca: file names, as Oeca's src/nearby.rs (safe_name, dangerous)

/** Shows nothing, or reorders what is shown: bidi controls, zero-width and filler characters, tags. */
const INVISIBLE =
  /[\u00AD\u034F\u061C\u115F\u1160\u17B4\u17B5\u180B-\u180F\u200B-\u200F\u202A-\u202E\u2060-\u206F\u3164\uFEFF\uFFA0\uFFF9-\uFFFB]|\uD834[\uDD73-\uDD7A]|\uDB40[\uDC00-\uDC7F]/g;
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001F\u007F-\u009F]/g;
const ILLEGAL = /[<>:"/\\|?*]/g;
/** UTF-16 units, as on the Windows side: room for " (999)" in a 255-unit name. */
export const MAX_SAVED_NAME = 235;
const MAX_EXTENSION = 16;

function isDeviceName(name: string): boolean {
  const stem = name.split(".")[0]!.replace(/[ .]+$/, "").toLowerCase();
  return /^(con|prn|aux|nul|conin\$|conout\$|clock\$|(com|lpt)[0-9\u00B9\u00B2\u00B3])$/.test(stem);
}

/** At most MAX_SAVED_NAME UTF-16 units and 255 UTF-8 bytes (the native side's limits), cut before the extension. */
function fit(name: string): string {
  const fits = (s: string) => s.length <= MAX_SAVED_NAME && new TextEncoder().encode(s).length <= 255;
  if (fits(name)) return name;
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 && [...name.slice(dot)].length <= MAX_EXTENSION + 1 ? name.slice(dot) : "";
  let stem = "";
  for (const c of name.slice(0, name.length - ext.length)) {
    if (!fits(stem + c + ext)) break;
    stem += c;
  }
  return stem.replace(/[ .]+$/, "") + ext;
}

/**
 * A file name that is safe to save and shows what it is: no control, bidi or
 * invisible characters (the name shown is the name saved, its real extension
 * last), no separators or streams, no Windows device names, no trailing dots
 * or spaces; too long, it is shortened before its extension.
 */
export function safeName(untrusted: string): string {
  let n = fit(untrusted.normalize("NFC").replace(CONTROL, "").replace(INVISIBLE, "").trim())
    .replace(ILLEGAL, "_")
    .replace(/[ .]+$/, (run) => (run ? "_" : ""));
  // As the core does for its list (the stem becomes "_"), then the names it misses
  const stem = n.split(".")[0]!;
  if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i.test(stem)) n = `_${n.slice(stem.length)}`;
  else if (isDeviceName(n)) n = `_${n}`;
  n = fit(n);
  // As the core's placeholder (and nothing left can be "." or "..": trailing dots became "_")
  return n === "" ? "untitled" : n;
}

const DANGEROUS = new Set(
  ("exe com scr pif cpl dll ocx sys drv msi msp mst msix msixbundle appx appxbundle appinstaller application appref-ms " +
    "bat cmd ps1 psm1 psd1 ps1xml psc1 vbs vbe js jse wsf wsh wsc hta msc jar reg inf chm hlp lnk url website scf " +
    "library-ms searchconnector-ms settingcontent-ms diagcab gadget xll xlam xlsm docm dotm pptm potm ppsm iso img vhd " +
    "vhdx theme themepack desktopthemepackfile shs ws apk command sh app dmg pkg").split(" "),
);

/** A type that runs code, or opens something elsewhere, when opened (its last extension). */
export function dangerous(name: string): boolean {
  const dot = name.lastIndexOf(".");
  return dot >= 0 && DANGEROUS.has(name.slice(dot + 1).toLowerCase());
}

/** Shown as the stem (which may be shortened on screen) and the extension (never hidden). */
export function nameParts(name: string): { stem: string; ext: string } {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? { stem: name.slice(0, dot), ext: name.slice(dot) } : { stem: name, ext: "" };
}
