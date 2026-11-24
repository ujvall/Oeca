/**
 * Oeca Nearby: who is at the other end of a connection, in types that cannot
 * be mixed up (the same as Oeca's src/nearby/proof.rs).
 *
 * - ClaimedFingerprint: read from signaling. Anyone can write one: it can be
 *   compared, never trusted.
 * - Connected: the key of the connection DTLS really made, equal to the claim.
 * - Authenticated: a Connected peer that completed commit-reveal and stated,
 *   inside that connection, the room id of the peer we meant. Only this can be
 *   pinned or shown under a trusted name.
 *
 * The channel "oeca-nearby-v1", opened by the offerer (ids and the
 * acknowledgement travel beside the unchanged construction, see OSV):
 * 1. answerer: {"ready":1,"peer":"<its room id>"}
 * 2. offerer, if that is the id it sent the offer to and the key is the
 *    claimed one: {"r":"<secret>","peer":"<its room id>"}
 * 3. answerer, if the secret opens the commitment and that is the id the
 *    offer came from: {"ok":1}
 */
import { certificateDigest, commitOf, fingerprintFromSdp, isSecret, OSV, sdpAttr, verificationCode } from "./trust";

export type FailureCode =
  | "busy"
  | "too-many-attempts"
  | "authentication"
  | "downgraded"
  | "revoked"
  | "trust-unavailable"
  | "other";

export class Failure extends Error {
  constructor(
    readonly code: FailureCode,
    message: string = code,
  ) {
    super(message);
  }
}

export type Capability = "unknown" | "stock" | "oeca";

// Only this module can make the three below
const MADE_HERE = Symbol("proof");

function onlyHere(made: symbol) {
  if (made !== MADE_HERE) throw new TypeError("An identity cannot be made outside the proof");
}

export class ClaimedFingerprint {
  readonly #value: string;
  constructor(made: symbol, value: string) {
    onlyHere(made);
    this.#value = value;
  }
  get claimed() {
    return this.#value;
  }
}

export class Connected {
  readonly #fingerprint: string;
  constructor(made: symbol, fingerprint: string) {
    onlyHere(made);
    this.#fingerprint = fingerprint;
  }
  get fingerprint() {
    return this.#fingerprint;
  }
}

export class Authenticated {
  readonly #connected: Connected;
  readonly #code: string;
  constructor(made: symbol, connected: Connected, code: string) {
    onlyHere(made);
    this.#connected = connected;
    this.#code = code;
  }
  get fingerprint() {
    return this.#connected.fingerprint;
  }
  get code() {
    return this.#code;
  }
}

/** What an offer or answer says. Claims only. */
export type Said = {
  fingerprint: ClaimedFingerprint | null;
  capability: "stock" | "oeca";
  /** The commitment (offer) or nonce (answer), if it is well formed. */
  value: string | null;
  busy: boolean;
};

export function readSdp(sdp: string, name: string): Said {
  const fp = fingerprintFromSdp(sdp);
  return {
    fingerprint: fp ? new ClaimedFingerprint(MADE_HERE, fp) : null,
    // Malformed is still Oeca (failed), never a reason to fall back
    capability: sdp.toLowerCase().includes("x-oeca-nearby") ? "oeca" : "stock",
    value: sdpAttr(sdp, name),
    busy: sdp.split(/\r?\n/).some((l) => l.trim() === `a=${OSV.BUSY}:1`),
  };
}

/** `value` null: a stock peer. */
export function usable(said: Said): { claimed: ClaimedFingerprint; value: string | null } {
  const oeca = said.capability === "oeca";
  const code = oeca ? "authentication" : "other";
  if (!said.fingerprint) throw new Failure(code, "No single certificate fingerprint was announced");
  if (oeca && !said.value) throw new Failure(code, "The verification data is malformed");
  return { claimed: said.fingerprint, value: oeca ? said.value : null };
}

/** Call only once a channel of `pc` is open. */
export async function connected(pc: RTCPeerConnection, claimed: ClaimedFingerprint): Promise<Connected> {
  const bad = new Failure("authentication", "The connection's certificate is not the announced one");
  // Browsers connect only to the key the applied SDP names
  const applied = pc.remoteDescription && fingerprintFromSdp(pc.remoteDescription.sdp);
  if (!applied || applied !== claimed.claimed) throw bad;
  // Where the certificate itself is exposed, it is checked as well
  const transport = pc.sctp?.transport;
  if (transport?.getRemoteCertificates) {
    const der = transport.getRemoteCertificates()[0];
    if (!der || `sha-256 ${await certificateDigest(der)}` !== applied) throw bad;
  }
  return new Connected(MADE_HERE, applied);
}

const OTHER_PEER = "The connection belongs to another device than the one selected (possible relay)";
const BAD_REVEAL =
  "Verification failed: the sender's reveal is missing or does not match its commitment (possible interception)";
const NO_PROOF = "Verification did not complete";

export type Exchange = {
  myId: string;
  /** The peer this side selected (send) or the offer came from (receive). */
  peerId: string;
  claimed: ClaimedFingerprint;
};

/** The exchange is three messages: more are dropped. */
function inbox(ch: RTCDataChannel) {
  const queued: string[] = [];
  let waiting: { resolve(m: string): void; reject(e: Error): void } | null = null;
  let closed = false;
  ch.onmessage = (e) => {
    if (typeof e.data !== "string" || e.data.length > 1024) return;
    if (waiting) waiting.resolve(e.data);
    else if (queued.length < 4) queued.push(e.data);
    waiting = null;
  };
  ch.onclose = () => {
    closed = true;
    waiting?.reject(new Failure("authentication", NO_PROOF));
    waiting = null;
  };
  return async (): Promise<Record<string, unknown>> => {
    const raw =
      queued.shift() ??
      (await new Promise<string>((resolve, reject) => {
        if (closed) reject(new Failure("authentication", NO_PROOF));
        else waiting = { resolve, reject };
      }));
    try {
      return Object(JSON.parse(raw));
    } catch {
      throw new Failure("authentication", NO_PROOF);
    }
  };
}

async function authenticated(pc: RTCPeerConnection, peer: Connected, secret: string, nonce: string) {
  const mine = pc.localDescription && fingerprintFromSdp(pc.localDescription.sdp);
  if (!mine) throw new Failure("authentication", NO_PROOF);
  return new Authenticated(MADE_HERE, peer, await verificationCode(mine, peer.fingerprint, secret, nonce));
}

/** Offerer, as soon as the answer is applied. */
export async function initiate(
  pc: RTCPeerConnection,
  x: Exchange,
  secret: string,
  nonce: string,
): Promise<Authenticated> {
  const ch = pc.createDataChannel(OSV.CHANNEL);
  const next = inbox(ch);
  const ready = await next();
  if (ready.ready !== 1) throw new Failure("authentication", NO_PROOF);
  if (ready.peer !== x.peerId) throw new Failure("authentication", OTHER_PEER);
  // The connection is up (it carried that). Reveal only to the claimed key
  const peer = await connected(pc, x.claimed);
  ch.send(JSON.stringify({ r: secret, peer: x.myId }));
  if ((await next()).ok !== 1) throw new Failure("authentication", NO_PROOF);
  return authenticated(pc, peer, secret, nonce);
}

/** Answerer. */
export async function respondToExchange(
  pc: RTCPeerConnection,
  channel: Promise<RTCDataChannel>,
  x: Exchange,
  commit: string,
  nonce: string,
): Promise<Authenticated> {
  const ch = await channel;
  const next = inbox(ch);
  if (ch.readyState !== "open") {
    await new Promise<void>((resolve) => ch.addEventListener("open", () => resolve(), { once: true }));
  }
  const peer = await connected(pc, x.claimed);
  ch.send(JSON.stringify({ ready: 1, peer: x.myId }));
  const reveal = await next();
  if (!isSecret(reveal.r) || (await commitOf(reveal.r)) !== commit) throw new Failure("authentication", BAD_REVEAL);
  if (reveal.peer !== x.peerId) throw new Failure("authentication", OTHER_PEER);
  ch.send(JSON.stringify({ ok: 1 }));
  return authenticated(pc, peer, reveal.r, nonce);
}
