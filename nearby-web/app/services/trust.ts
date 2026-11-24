/**
 * Oeca Nearby: this browser's identity, and the pure parts of verification.
 *
 * Identity is the DTLS certificate of the WebRTC connection, not the alias
 * and not LocalSend's signed token (which is not bound to the connection).
 * This browser keeps one certificate in IndexedDB so its fingerprint stays
 * the same across visits.
 *
 * Who the other device is: proof.ts. Whom this user trusts: devices.ts. What
 * may be decided when: transfer.ts.
 */

import { decodeBase64, encodeBase64 } from "../utils/base64";

const DB = "oeca-nearby";
const STORE = "identity";
/** Asked lifetime; browsers may cap it (see OECA.md). */
const CERT_LIFETIME_MS = 365 * 24 * 3600 * 1000;
/** From then on the page says when this identity ends. */
const NOTICE_BEFORE_MS = 30 * 24 * 3600 * 1000;

function idb<T>(mode: IDBTransactionMode, run: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => open.result.createObjectStore(STORE);
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const tx = open.result.transaction(STORE, mode);
      const req = run(tx.objectStore(STORE));
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
      tx.oncomplete = () => open.result.close();
    };
  });
}

/**
 * What happened to this browser's identity on this visit:
 * - kept: the stored certificate (same fingerprint as before);
 * - new: none was stored (first visit, or site data cleared): devices that
 *   knew this browser see a new or changed device and ask to verify;
 * - renewed: the stored one had expired and a new one replaced it (same effect);
 * - unsaved: storage failed (private mode...): a new identity every visit.
 * Never silent: the page says it (index.vue), and peers never trust a new
 * fingerprint without a verification.
 */
export type IdentityState = "kept" | "new" | "renewed" | "unsaved";

export async function loadCertificate(): Promise<{
  cert: RTCCertificate;
  state: IdentityState;
  expires: number;
  endingSoon: boolean;
}> {
  let state: IdentityState = "new";
  try {
    const stored = await idb<RTCCertificate | undefined>("readonly", (s) => s.get("dtls"));
    if (stored && stored.expires > Date.now()) {
      // Kept until it really expires: an early renewal would be a silent
      // identity change for every device that verified this one
      return {
        cert: stored,
        state: "kept",
        expires: stored.expires,
        endingSoon: stored.expires - Date.now() < NOTICE_BEFORE_MS,
      };
    }
    if (stored) state = "renewed";
  } catch (e) {
    console.warn("No stored certificate:", (e as Error)?.name);
    state = "unsaved";
  }
  const cert = await RTCPeerConnection.generateCertificate({
    name: "ECDSA",
    namedCurve: "P-256",
    expires: CERT_LIFETIME_MS,
  } as AlgorithmIdentifier);
  try {
    if (state === "unsaved") throw new Error("storage unavailable");
    const kept = await claim(cert);
    // Another tab stored its new one first: that one is this browser's identity
    if (kept !== cert) return { cert: kept, state: "kept", expires: kept.expires, endingSoon: false };
  } catch (e) {
    console.warn("Certificate not kept:", (e as Error)?.name);
    state = "unsaved";
  }
  return { cert, state, expires: cert.expires, endingSoon: false };
}

/**
 * oeca: stores `fresh` unless a valid certificate is stored by now, in one
 * transaction (the browser runs one tab's at a time), and resolves only once
 * it is committed: two tabs opened together end with one identity, and a
 * write that did not commit is never taken as kept.
 */
function claim(fresh: RTCCertificate): Promise<RTCCertificate> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => open.result.createObjectStore(STORE);
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const tx = open.result.transaction(STORE, "readwrite");
      const store = tx.objectStore(STORE);
      let chosen = fresh;
      const read = store.get("dtls");
      read.onsuccess = () => {
        const stored = read.result as RTCCertificate | undefined;
        if (stored && stored.expires > Date.now()) chosen = stored;
        else store.put(fresh, "dtls");
      };
      tx.oncomplete = () => {
        open.result.close();
        resolve(chosen);
      };
      tx.onabort = tx.onerror = () => {
        open.result.close();
        reject(tx.error ?? new Error("not saved"));
      };
    };
  });
}

/**
 * oeca: this browser's identity (its certificate) and its alias, deleted: the
 * next visit makes new ones. Nothing else is touched (see resetIdentity).
 */
export async function deleteIdentity(): Promise<void> {
  await idb("readwrite", (s) => s.delete("dtls"));
  try {
    localStorage.removeItem("oeca-nearby-alias");
  } catch {
    // no storage: there was no alias kept either
  }
}

/**
 * A stable name for this browser (upstream picks a new one per visit, which
 * would hide "same name, other certificate" from peers).
 */
export function stableAlias(fresh: () => string): string {
  try {
    const kept = localStorage.getItem("oeca-nearby-alias");
    if (kept) return kept;
    const alias = fresh();
    localStorage.setItem("oeca-nearby-alias", alias);
    return alias;
  } catch {
    return fresh();
  }
}

/**
 * The SDP's one fingerprint, "sha-256 ab:cd:…" (lower case, 32 bytes), or null.
 * Parsers differ on which line wins, so anything ambiguous is refused: every
 * line that mentions a fingerprint must be exactly a=fingerprint: with the
 * same value (the same rule as Oeca's src/nearby.rs).
 */
export function fingerprintFromSdp(sdp: string): string | null {
  let found: string | null = null;
  for (const line of sdp.split(/[\r\n]/)) {
    if (!line.toLowerCase().includes("fingerprint")) continue;
    if (!line.startsWith("a=fingerprint:")) return null;
    const fp = canonicalFingerprint(line.slice("a=fingerprint:".length));
    if (!fp) return null;
    if (found && found !== fp) return null;
    found = fp;
  }
  return found;
}

/** "sha-256 ab:cd:…" (32 lower-case bytes), or null. The one form a key is stored or compared in. */
export function canonicalFingerprint(text: string): string | null {
  // oeca: ASCII whitespace and case only, as src/nearby/proof.rs (Fingerprint::parse)
  const fp = text.split(/[ \t\n\f\r]+/).filter(Boolean).join(" ").replace(/[A-Z]/g, (c) => c.toLowerCase());
  // Only SHA-256 (what browsers and webrtc-rs use): weaker digests are refused
  return /^sha-256 [0-9a-f]{2}(:[0-9a-f]{2}){31}$/.test(fp) ? fp : null;
}

/**
 * Oeca Nearby Verification v1 (the same as Oeca's `osv` module in
 * src/nearby.rs; both test the same vector). LocalSend's messages are
 * untouched; peers without it see two ignored SDP lines.
 * 1. Offer: a=x-oeca-nearby-commit:C, C = b64(SHA-256("oeca-nearby-commit-v1|" + rO)).
 * 2. Answer, only if the offer had C: a=x-oeca-nearby-nonce:rA. A peer that
 *    cannot take the transfer answers a=x-oeca-nearby-busy:1, with no nonce.
 * 3. Only if the answer had rA: the offerer opens the channel "oeca-nearby-v1",
 *    where rO is revealed and checked against C (proof.ts has the messages).
 * 4. Code: six digits of SHA-256("oeca-nearby-code-v2|" + lower fp + "|" +
 *    higher fp + "|" + rO + "|" + rA). Each random is fixed before the other
 *    side's is seen: someone in the middle gets one blind guess.
 */
export const OSV = {
  COMMIT: "x-oeca-nearby-commit",
  NONCE: "x-oeca-nearby-nonce",
  BUSY: "x-oeca-nearby-busy",
  CHANNEL: "oeca-nearby-v1",
};

async function sha256(text: string): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
}

export function randomSecret(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return encodeBase64(bytes);
}

export async function commitOf(secret: string): Promise<string> {
  return encodeBase64(await sha256(`oeca-nearby-commit-v1|${secret}`));
}

/**
 * oeca: a secret is 32 bytes as canonical unpadded URL-safe base64 (43
 * characters), exactly what the native side's decoder accepts.
 */
export function isSecret(value: unknown): value is string {
  if (typeof value !== "string" || !/^[A-Za-z0-9_-]{43}$/.test(value)) return false;
  try {
    return encodeBase64(decodeBase64(value)) === value;
  } catch {
    return false;
  }
}

/**
 * The one session-level a=name:value holding a secret, or null. One grammar
 * with Oeca's src/nearby.rs (osv::attr; shared vectors in both tests): lines
 * end at LF (a CR before it is dropped), nothing is trimmed, and the line
 * appears exactly once, before the first m=.
 */
export function sdpAttr(sdp: string, name: string): string | null {
  const prefix = `a=${name}:`;
  const lines = sdp.split("\n").map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l));
  const media = lines.findIndex((l) => l.startsWith("m="));
  const at = lines.flatMap((l, i) => (l.startsWith(prefix) ? [i] : []));
  if (at.length !== 1 || (media >= 0 && at[0]! > media)) return null;
  const value = lines[at[0]!]!.slice(prefix.length);
  return isSecret(value) ? value : null;
}

/** Adds session-level lines before the first m= line (where the native side adds them). */
export function withSessionLines(sdp: string, lines: string[]): string {
  if (!lines.length) return sdp;
  const added = lines.map((l) => `${l}\r\n`).join("");
  const i = sdp.indexOf("\nm=");
  return i < 0 ? sdp + added : sdp.slice(0, i + 1) + added + sdp.slice(i + 1);
}

export async function verificationCode(a: string, b: string, offerSecret: string, answerNonce: string): Promise<string> {
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  const digest = await sha256(`oeca-nearby-code-v2|${lo}|${hi}|${offerSecret}|${answerNonce}`);
  const n = new DataView(digest.buffer).getUint32(0) % 1_000_000;
  return `${String(Math.floor(n / 1000)).padStart(3, "0")} ${String(n % 1000).padStart(3, "0")}`;
}

export async function certificateDigest(der: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", der));
  return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join(":");
}
