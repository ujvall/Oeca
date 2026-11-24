// oeca: what the page shows, as plain functions of the store's state (no
// Vue, no network), so each rule has a test. Decisions are never made here:
// the page passes the transfer id it showed to store.ts, which checks it.

import type { Stage, TransferView } from "~/services/transfer";
import type { FileState, Link } from "~/services/store";
import { dangerous } from "~/services/limits";
import { formatBytes } from "~/utils/fileSize";

export type Tone = "plain" | "good" | "warn" | "bad";

/** The header's status. */
export function statusLine(s: {
  framed: boolean;
  crypto: boolean;
  started: boolean;
  link: Link;
  online: boolean;
  limited: boolean;
  peers: number;
}): { label: string; tone: Tone; busy: boolean } {
  if (s.framed) return { label: "Unavailable", tone: "bad", busy: false };
  if (!s.crypto) return { label: "Not supported", tone: "bad", busy: false };
  if (!s.started) return { label: "Off", tone: "plain", busy: false };
  if (s.online) {
    const n = s.peers;
    return { label: n === 0 ? "Searching" : `${n} nearby`, tone: "good", busy: n === 0 };
  }
  if (s.limited) return { label: "Server busy", tone: "warn", busy: false };
  return s.link === "reconnecting"
    ? { label: "Reconnecting", tone: "warn", busy: true }
    : { label: "Connecting", tone: "plain", busy: true };
}

/** A peer's kind, for its icon and detail line (what it says it is: a claim). */
export function peerKind(deviceType: string | undefined): { icon: "phone" | "computer" | "browser" | "device"; label: string } {
  switch (deviceType) {
    case "mobile":
      return { icon: "phone", label: "Phone" };
    case "desktop":
      return { icon: "computer", label: "Computer" };
    case "web":
      return { icon: "browser", label: "Browser" };
    case "headless":
    case "server":
      return { icon: "device", label: "Device" };
    default:
      return { icon: "device", label: "Unknown device" };
  }
}

/** A device's own description, as plain words ("Windows computer"). A claim. */
export function peerDetail(deviceType: string | undefined, model: string | null | undefined): string {
  const kind = peerKind(deviceType).label;
  const m = (model ?? "").trim();
  if (!m) return kind;
  return kind === "Unknown device" ? m : `${m} ${kind.toLowerCase()}`;
}

/** What the notch shows: the connection, or the transfer while there is one. */
export type NotchMode = "off" | "quiet" | "searching" | "nearby" | "warn" | "bad" | "asks" | "moving" | "done" | "ended";

export function notchState(
  status: { label: string; tone: Tone; busy: boolean },
  started: boolean,
  t: TransferView | null,
  files: Record<string, FileState>,
): { mode: NotchMode; label: string; progress: number | null } {
  if (t) {
    const receive = t.direction === "receive";
    if (ASKING.includes(t.stage)) return { mode: "asks", label: t.stage === "approval" ? "Incoming files" : "Check this device", progress: null };
    if (t.stage === "transferring") {
      const { done, all } = confirmedCount(files);
      return {
        mode: "moving",
        label: `${receive ? "Receiving" : "Sending"} ${done} of ${all}`,
        progress: overallProgress(files, t.direction),
      };
    }
    if (t.stage === "done") return { mode: "done", label: receive ? "Received" : "Sent", progress: 1 };
    if (FINAL.includes(t.stage)) return { mode: "ended", label: t.stage === "failed" ? "Didn't finish" : t.stage === "declined" ? "Declined" : "Cancelled", progress: null };
    return { mode: "quiet", label: "Connecting", progress: null };
  }
  if (!started) return { mode: status.tone === "bad" ? "bad" : "off", label: status.tone === "bad" ? status.label : "Hop is off", progress: null };
  if (status.tone === "bad") return { mode: "bad", label: status.label, progress: null };
  if (status.tone === "warn") return { mode: "warn", label: status.label, progress: null };
  if (status.label === "Searching") return { mode: "searching", label: "Searching", progress: null };
  if (status.tone === "good") return { mode: "nearby", label: status.label, progress: null };
  return { mode: "quiet", label: status.label, progress: null };
}

/** Who the other device is, from the transfer's proven identity only. */
export function identityBadge(t: TransferView): { label: string; tone: Tone } | null {
  const id = t.identity?.state;
  if (id === "trusted") return { label: "Trusted", tone: "good" };
  if (id === "changed") return { label: "Identity changed", tone: "bad" };
  if (id === "revoked") return { label: "Forgotten device", tone: "bad" };
  if (id === "new" && t.capability === "stock") return { label: "Unverifiable", tone: "warn" };
  if (id === "new") return { label: "New device", tone: "plain" };
  if (t.connection.state === "closed") return null;
  return { label: "Not verified yet", tone: "plain" };
}

/** The name to show: the one the user confirmed for a trusted device, otherwise its own claim, quoted. */
export function peerName(t: TransferView): string {
  const i = t.identity;
  if (i?.state === "trusted") return i.name;
  return `“${t.claimed.peer.alias || "Unnamed device"}”`;
}

/** The sheet's heading for a stage. */
export function stageTitle(t: TransferView): string {
  const receiving = t.direction === "receive";
  switch (t.stage) {
    case "signaling":
      return "Waiting for the device to answer…";
    case "connecting":
      return "Connecting…";
    case "verification":
      return "Compare the code";
    case "name-check":
      return "Is this the same device?";
    case "unverified":
      return "Send without verification?";
    case "awaiting-offer":
      return "Waiting for the list of files…";
    case "approval":
      return "Incoming files";
    case "awaiting-peer":
      return "Waiting for the other device to accept…";
    case "transferring":
      return receiving ? "Receiving…" : "Sending…";
    case "done":
      return receiving ? "Received" : "Sent";
    case "declined":
      return receiving ? "You declined" : "Declined by the other device";
    case "cancelled":
      return "Cancelled";
    case "failed":
      return receiving ? "Couldn't receive" : "Couldn't send";
  }
}

export const ASKING: readonly Stage[] = ["verification", "name-check", "unverified", "approval"];
export const FINAL: readonly Stage[] = ["done", "declined", "cancelled", "failed"];

/** When the question on screen arms (`null`: not yet shown). Decisions before it are refused. */
export function armsIn(t: TransferView, now: number): number | null {
  if (!ASKING.includes(t.stage)) return 0;
  if (t.decideAfter === null) return null;
  return Math.max(0, t.decideAfter - now);
}

/** One file's line: what is known about it, never more. */
export type FileRow = {
  id: string;
  name: string;
  size: string;
  label: string;
  tone: Tone;
  /** 0..1 while bytes move; null when there is nothing to show. */
  progress: number | null;
  /** Confirmed: saved by the receiver (send), or arrived whole (receive). */
  done: boolean;
};

export function fileRows(files: Record<string, FileState>, direction: "send" | "receive", stage: Stage): FileRow[] {
  const ended = FINAL.includes(stage);
  return Object.values(files).map((f) => {
    const share = f.total > 0 ? Math.min(1, f.curr / f.total) : 0;
    let label: string;
    let tone: Tone = "plain";
    let progress: number | null = null;
    switch (f.state) {
      case "finished":
        label = direction === "send" ? "Saved by the other device" : "Received";
        tone = "good";
        break;
      case "skipped":
        label = "Not wanted by the other device";
        break;
      case "error":
        label = f.error ? `Not saved: ${f.error}` : "Not saved";
        tone = "bad";
        break;
      default:
        if (ended) {
          label = "Not confirmed";
          tone = "warn";
        } else if (direction === "send" && f.total > 0 && f.curr >= f.total) {
          // Every byte left, the receiver has not said it saved them
          label = "Waiting for confirmation";
          progress = 1;
        } else if (f.curr > 0) {
          label = `${Math.floor(share * 100)}%`;
          progress = share;
        } else {
          label = "Queued";
        }
    }
    return { id: f.id, name: f.name, size: formatBytes(f.total), label, tone, progress, done: f.state === "finished" };
  });
}

/**
 * Overall progress, confirmed only: sending counts a file once the receiver
 * said it saved it (bytes sent are only queued until then); receiving counts
 * what arrived here. Files with no bytes count by number.
 */
export function overallProgress(files: Record<string, FileState>, direction: "send" | "receive"): number {
  const all = Object.values(files);
  if (all.length === 0) return 0;
  const total = all.reduce((a, f) => a + f.total, 0);
  if (total === 0) return all.filter((f) => f.state === "finished").length / all.length;
  const done = all.reduce((a, f) => {
    if (f.state === "finished") return a + f.total;
    if (direction === "receive" && f.state !== "error" && f.state !== "skipped") return a + Math.min(f.curr, f.total);
    return a;
  }, 0);
  return Math.min(1, done / total);
}

/** "3 of 5 files" confirmed. */
export function confirmedCount(files: Record<string, FileState>): { done: number; all: number } {
  const all = Object.values(files);
  return { done: all.filter((f) => f.state === "finished").length, all: all.length };
}

/** Files to send: at most this many at once (what a receiving browser accepts). */
export const MAX_SEND_FILES = 100;

/** What stops (or warns about) sending the chosen files. */
export function selectionIssues(files: readonly { name: string; size: number }[]): {
  blocking: string | null;
  notes: string[];
} {
  if (files.length === 0) return { blocking: "Choose at least one file.", notes: [] };
  const notes: string[] = [];
  const risky = files.filter((f) => dangerous(f.name)).length;
  if (risky) {
    notes.push(`${risky} ${risky === 1 ? "file can" : "files can"} run programs or open other places on the receiving device.`);
  }
  const empty = files.filter((f) => f.size === 0).length;
  if (empty) notes.push(`${empty} empty ${empty === 1 ? "file" : "files"} (0 bytes) will be sent as ${empty === 1 ? "it is" : "they are"}.`);
  if (files.length > MAX_SEND_FILES) {
    return { blocking: `Send up to ${MAX_SEND_FILES} files at a time (${files.length} chosen).`, notes };
  }
  return { blocking: null, notes };
}

export function totalSize(files: readonly { size: number }[]): string {
  return formatBytes(files.reduce((a, f) => a + f.size, 0));
}

export function countLabel(n: number): string {
  return `${n} ${n === 1 ? "file" : "files"}`;
}

/** The privacy notice, in the order it is read. Its id (a hash of this text) records what was accepted. */
export function privacyNotice(alias: string): { title: string; text: string }[] {
  return [
    {
      title: "Public servers",
      text:
        "While Oeca Hop is on, this page connects to LocalSend's public signaling server (public.localsend.org) " +
        "and STUN server (stun.localsend.org), run by the LocalSend project, not by Oeca. Both see your public " +
        `IP address. The signaling server also sees this browser's name (“${alias}”), its device type and ` +
        "browser, and passes on every connection request, which carries this browser's Hop identity and its " +
        "network addresses.",
    },
    {
      title: "Who can see you",
      text:
        "The signaling server lists this browser to every device that shares your public IPv4 address or IPv6 /64 " +
        "network: usually your own Wi-Fi, but on public Wi-Fi, a VPN or a mobile network that can include strangers. " +
        "Any of them can see this browser's name and ask to connect, but nothing reaches you without your approval.",
    },
    {
      title: "Networks",
      text:
        "A VPN, and whether your devices connect over IPv4 or IPv6, decide which devices find each other. Hop " +
        "doesn't promise that only devices on your Wi-Fi can see you. When the two devices can't reach each other " +
        "directly, the transfer fails: nothing is relayed.",
    },
    {
      title: "Your files",
      text:
        "Files go directly between the two devices over an encrypted WebRTC connection, never through those " +
        "servers or through Oeca. Nothing is received without your approval. Received files stay in this " +
        "browser's private storage until you save them, and are never opened automatically. Saving uses your " +
        "browser's downloads.",
    },
    {
      title: "This website",
      text:
        "Like any website, the host that serves this page (Vercel) can log requests to it: your IP address, the time and " +
        "your browser. This browser keeps its Hop identity and your trusted devices in this site's storage; " +
        "clearing the site's data removes them.",
    },
  ];
}

/** FNV-1a of the notice's text (without the alias): a changed notice is asked for again. */
export function noticeId(): string {
  let h = 0x811c9dc5;
  for (const part of privacyNotice("")) {
    for (const c of part.title + part.text) {
      h = Math.imul(h ^ c.codePointAt(0)!, 0x01000193) >>> 0;
    }
  }
  return h.toString(16);
}
