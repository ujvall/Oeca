/**
 * Oeca Nearby: the decision API's state machine (the same as Oeca's
 * src/nearby/transfer.rs). It is the only place a transfer's identity, stage
 * or decision can change.
 *
 * send:    signaling -> connecting -> verification | name-check | unverified | (trusted)
 *                    -> awaiting-peer -> transferring
 * receive: connecting -> verification | name-check | (trusted) | (stock)
 *                    -> awaiting-offer -> approval -> transferring
 * any:     -> done | declined | cancelled | failed (final)
 *
 * - A decision names its transfer's id (random, made here, never sent). The
 *   store is asynchronous, so the id and stage are checked again after every wait.
 * - It exists only in the stage that asks for it.
 * - It counts only ARMING_MS after that stage was on screen (`displayed`).
 * - Identity, names and trust come only from proof.ts's classes, and are
 *   cleared when a transfer ends without succeeding.
 */
import { cleanName, type Devices, type Change, type Standing, TrustError } from "./devices";
import { Authenticated, type Capability, type ClaimedFingerprint, Connected, Failure, type FailureCode } from "./proof";
import { randomSecret } from "./trust";

/** How long a question is on screen before an answer to it counts. */
export const ARMING_MS = 700;
/** No decision within this declines (as the native side does). */
export const DECISION_TIMEOUT_MS = 120_000;
/** The other user compares, then decides. */
const PEER_DECIDES_MS = 2 * DECISION_TIMEOUT_MS + 10_000;
/** Gathering, the answer or connection, and the handshake. */
const CONNECT_MS = 75_000;
/** A stock peer sends its list at once (or after its user typed a PIN). */
const STOCK_LIST_MS = 60_000;
const IDLE_RECEIVE_MS = 30_000;
/** A sender counts what it queued, up to 1 MiB ahead of the wire. */
const IDLE_SEND_MS = 120_000;
/** The idle deadline restarts only after this much moved: a drip is not progress. */
export const MIN_PROGRESS = 64 * 1024;
/** After the end, how long the connection may take to say so and close. */
const CLOSE_GRACE_MS = 6000;
const MAX_WHY = 300;

export type Direction = "send" | "receive";

export type Stage =
  | "signaling"
  | "connecting"
  | "verification"
  | "name-check"
  | "unverified"
  | "awaiting-offer"
  | "awaiting-peer"
  | "approval"
  | "transferring"
  | "done"
  | "declined"
  | "cancelled"
  | "failed";

const ASKS: readonly Stage[] = ["verification", "name-check", "unverified", "approval"];

export function isFinal(stage: Stage) {
  return stage === "done" || stage === "declined" || stage === "cancelled" || stage === "failed";
}

/** Who it is, decided only for a connected key. Separate from the capability. */
export type Identity =
  | { state: "new" }
  /** Only ever with an authenticated connection. Show `name`, never the live alias. */
  | { state: "trusted"; name: string; nameMismatch: boolean }
  | { state: "changed" }
  | { state: "revoked" };

/** What is proven about the connection. */
export type Connection =
  | { state: "claimed" }
  | { state: "connected"; fingerprint: string }
  | { state: "authenticated"; fingerprint: string; code: string }
  | { state: "closed" };

export type Offered = { id: string; fileName: string; size: number };

/** What the page gets: a plain copy, changing it changes nothing. */
export type TransferView = {
  /** Every decision takes this. */
  id: string;
  direction: Direction;
  /** What signaling said: none of it proven, none of it a name to trust. */
  claimed: { peer: { id: string; alias: string }; fingerprint: string | null };
  capability: Capability;
  connection: Connection;
  identity: Identity | null;
  stage: Stage;
  failure: { code: FailureCode; message: string } | null;
  /** In a stage that asks: when answers start to count (performance.now()). */
  decideAfter: number | null;
  offered: Offered[];
};

export type RefusedCode =
  | "not-current"
  | "wrong-stage"
  | "not-yet"
  | "not-eligible"
  | "busy"
  | "offline"
  | "peer-changed"
  | "trust";

export class Refused extends Error {
  constructor(
    readonly code: RefusedCode,
    readonly trust?: TrustError,
  ) {
    super(trust ? trust.message : code);
  }
}

// Wrapped: an async function would flatten a bare promise into its answer
export type Gate = { answer: Promise<boolean> } | null;

type Question = { resolve(yes: boolean): void; timer: ReturnType<typeof setTimeout> };

type Current = {
  id: string;
  direction: Direction;
  peer: { id: string; alias: string };
  claimed: ClaimedFingerprint | null;
  capability: Capability;
  proof: Connected | Authenticated | null;
  closed: boolean;
  identity: Identity | null;
  stage: Stage;
  failure: Failure | null;
  offered: Offered[];
  abort: () => void;
  question: Question | null;
  watchdog: ReturnType<typeof setTimeout> | null;
  /** The total at the last progress that counted. */
  mark: number | null;
  seen: number | null;
  /** A decision is waiting for the store. */
  deciding: boolean;
  admitted: boolean;
};

function identityOf(standing: Standing): Identity | null {
  switch (standing.kind) {
    case "trusted":
      return { state: "trusted", name: standing.name, nameMismatch: standing.conflict };
    case "unavailable":
      return null;
    default:
      return { state: standing.kind };
  }
}

function sameKind(identity: Identity | null, standing: Standing) {
  return (identity?.state ?? "unavailable") === standing.kind;
}

export class Board {
  #t: Current | null = null;
  #closing: (() => void) | null = null;
  #devices: Devices;
  #publish: (view: TransferView | null) => void;
  #now: () => number;

  constructor(devices: Devices, publish: (view: TransferView | null) => void = () => {}, now = () => performance.now()) {
    this.#devices = devices;
    this.#publish = publish;
    this.#now = now;
    devices.onChange((change) => this.#storeChanged(change));
  }

  view(): TransferView | null {
    const t = this.#t;
    if (!t) return null;
    let connection: Connection = { state: t.closed ? "closed" : "claimed" };
    if (!t.closed && t.proof instanceof Authenticated) {
      connection = { state: "authenticated", fingerprint: t.proof.fingerprint, code: t.proof.code };
    } else if (!t.closed && t.proof) {
      connection = { state: "connected", fingerprint: t.proof.fingerprint };
    }
    return {
      id: t.id,
      direction: t.direction,
      claimed: { peer: { ...t.peer }, fingerprint: t.claimed?.claimed ?? null },
      capability: t.capability,
      connection,
      identity: t.identity && { ...t.identity },
      stage: t.stage,
      failure: t.failure && { code: t.failure.code, message: t.failure.message },
      decideAfter: t.seen === null ? null : t.seen + ARMING_MS,
      offered: t.offered.map((f) => ({ ...f })),
    };
  }

  #changed() {
    this.#publish(this.view());
  }

  busy(): boolean {
    return this.#t !== null && !isFinal(this.#t.stage);
  }

  isLive(id: string): boolean {
    return this.#live(id) !== null;
  }

  #live(id: string): Current | null {
    const t = this.#t;
    return t && t.id === id && !isFinal(t.stage) ? t : null;
  }

  /** A new attempt, with a new id. Whatever was there is over. */
  begin(direction: Direction, peer: { id: string; alias: string }, abort: () => void): string {
    if (this.#t && !isFinal(this.#t.stage)) this.kill(this.#t.id, "cancelled");
    this.#closing?.();
    this.#closing = null;
    const id = randomSecret();
    this.#t = {
      id,
      direction,
      peer: { id: peer.id, alias: cleanName(peer.alias) },
      claimed: null,
      capability: "unknown",
      proof: null,
      closed: false,
      identity: null,
      stage: direction === "send" ? "signaling" : "connecting",
      failure: null,
      offered: [],
      abort,
      question: null,
      watchdog: null,
      mark: null,
      seen: null,
      deciding: false,
      admitted: false,
    };
    this.#watch(this.#t);
    this.#changed();
    return id;
  }

  /** Takes a finished transfer off the screen. */
  clear() {
    if (this.busy()) return;
    this.#t = null;
    this.#changed();
  }

  #enter(t: Current, stage: Stage) {
    if (t.question) {
      clearTimeout(t.question.timer);
      t.question.resolve(false);
      t.question = null;
    }
    t.stage = stage;
    t.seen = null;
    t.deciding = false;
    this.#watch(t);
    this.#changed();
  }

  #deadline(t: Current): number | null {
    switch (t.stage) {
      case "signaling":
      case "connecting":
        return CONNECT_MS;
      case "awaiting-offer":
        return t.capability === "oeca" ? PEER_DECIDES_MS : STOCK_LIST_MS;
      case "awaiting-peer":
        return PEER_DECIDES_MS;
      case "transferring":
        return t.direction === "send" ? IDLE_SEND_MS : IDLE_RECEIVE_MS;
      default:
        return null; // a question has its own
    }
  }

  /** Every stage ends by itself, whatever the peer does. */
  #watch(t: Current) {
    if (t.watchdog !== null) clearTimeout(t.watchdog);
    t.watchdog = null;
    if (isFinal(t.stage)) {
      // done and declined finish over the protocol; not for ever
      t.watchdog = setTimeout(() => t.abort(), CLOSE_GRACE_MS);
      return;
    }
    const ms = this.#deadline(t);
    if (ms === null) return;
    t.watchdog = setTimeout(() => {
      this.kill(t.id, "failed", new Failure("other", "Timed out waiting for the other device"));
    }, ms);
  }

  /** `total` only grows. Too little since the last time is not progress. */
  progress(id: string, total: number) {
    const t = this.#live(id);
    if (!t || t.stage !== "transferring") return;
    if (t.mark !== null && total - t.mark < MIN_PROGRESS) return;
    t.mark = total;
    this.#watch(t);
  }

  /**
   * An incoming connection that has proven and asked nothing: the user's own
   * send may take its place.
   */
  unprovenIncoming(): string | null {
    const t = this.#t;
    if (!t || t.direction !== "receive") return null;
    const waiting = t.stage === "connecting" || (t.stage === "awaiting-offer" && t.capability === "stock");
    return waiting ? t.id : null;
  }

  #ask(t: Current, stage: Stage): Gate {
    this.#enter(t, stage);
    const answer = new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => {
        if (!this.#live(t.id) || t.question?.timer !== timer) return;
        // An unanswered offer is a decline, said over the protocol
        if (stage === "approval") this.end(t.id, "declined");
        else this.kill(t.id, "failed", new Failure("other", "Timed out waiting for a decision"));
      }, DECISION_TIMEOUT_MS);
      t.question = { resolve, timer };
    });
    return { answer };
  }

  #pass(t: Current) {
    t.admitted = true;
    this.#enter(t, t.direction === "send" ? "awaiting-peer" : "awaiting-offer");
  }

  #open(t: Current, yes: boolean) {
    if (!t.question) return;
    clearTimeout(t.question.timer);
    t.question.resolve(yes);
    t.question = null;
  }

  /** Signaling spoke. It sets only what is claimed. */
  claim(id: string, fingerprint: ClaimedFingerprint | null, capability: Capability) {
    const t = this.#live(id);
    if (!t || t.proof) return;
    t.claimed = fingerprint;
    t.capability = capability;
    this.#enter(t, "connecting");
  }

  /** The only way a transfer gets an identity. */
  async admit(id: string, proof: Connected | Authenticated): Promise<Gate> {
    const gone = () => new Failure("other", "The transfer is over");
    let t = this.#live(id);
    if (!t || t.proof) throw gone();
    const oeca = proof instanceof Authenticated;
    if (!oeca && !(proof instanceof Connected)) throw gone();
    const refuse = (failure: Failure) => {
      this.kill(id, "failed", failure);
      return failure;
    };
    // Offering Oeca and not completing it is not a stock peer
    if ((t.capability === "oeca") !== oeca) throw refuse(new Failure("authentication", "Incomplete"));
    const standing = await this.#devices.standing(proof.fingerprint, t.peer.alias);
    t = this.#live(id);
    if (!t || t.proof) throw gone();
    if (standing.kind === "unavailable") throw refuse(new Failure("trust-unavailable"));
    t.proof = proof;
    // Unauthenticated, a known key keeps its warnings and never its name
    t.identity = !oeca && standing.kind === "trusted" ? null : identityOf(standing);
    if (oeca) {
      if (standing.kind !== "trusted") return this.#ask(t, "verification");
      if (standing.conflict) return this.#ask(t, "name-check");
    } else if (standing.kind !== "new") {
      throw refuse(new Failure("downgraded"));
    } else if (t.direction === "send") {
      return this.#ask(t, "unverified");
    }
    this.#pass(t);
    return null;
  }

  /** The complete list: it enters the view together with the approval stage. */
  offer(id: string, files: Offered[]): Gate {
    const t = this.#live(id);
    if (!t || !t.admitted || t.stage !== "awaiting-offer") return null;
    t.offered = files.map((f) => ({ id: f.id, fileName: f.fileName, size: f.size }));
    return this.#ask(t, "approval");
  }

  /** The receiver took the offer. */
  sending(id: string) {
    const t = this.#live(id);
    if (t?.stage === "awaiting-peer") this.#enter(t, "transferring");
  }

  /** Before each thing that gives the peer something: still allowed? */
  async assertAllowed(id: string): Promise<void> {
    const over = () => new Failure("other", "The transfer is over");
    const t = this.#live(id);
    if (!t?.admitted || !t.proof) throw over();
    const standing = await this.#devices.standing(t.proof.fingerprint, t.peer.alias);
    if (!this.#live(id)) throw over();
    if (standing.kind === "unavailable") throw new Failure("trust-unavailable");
    const wanted = t.proof instanceof Authenticated ? "trusted" : "new";
    if (standing.kind !== wanted) throw new Failure("revoked");
  }

  #storeChanged(change: Change) {
    const t = this.#t;
    const key = !t?.closed && t?.proof?.fingerprint;
    if (t && key && change.revoked && (change.revoked.all || change.revoked.keys.includes(key))) {
      // At once, before the store has even written it
      if (!sameKind(t.identity, { kind: "revoked" })) {
        this.#withdraw(t, new Failure("revoked"));
        return;
      }
    }
    // A revocation's other effects are known once it is settled
    if (!change.revoked) void this.recheck();
  }

  /** The device store changed: end what it no longer allows. */
  async recheck(): Promise<void> {
    const t = this.#t;
    if (!t || t.closed || !t.proof) return;
    const shown = t.identity;
    const standing = await this.#devices.standing(t.proof.fingerprint, t.peer.alias);
    if (this.#t !== t || t.closed) return;
    if (t.identity !== shown) return this.recheck(); // decided meanwhile
    if (sameKind(shown, standing)) return;
    this.#withdraw(t, new Failure(standing.kind === "unavailable" ? "trust-unavailable" : "revoked"));
  }

  #withdraw(t: Current, failure: Failure) {
    if (isFinal(t.stage)) {
      t.closed = true;
      t.identity = null;
      this.#changed();
    } else {
      this.kill(t.id, "failed", failure);
    }
  }

  /** Final, once. Its decisions die with it. */
  end(id: string, stage: "done" | "declined" | "cancelled" | "failed", failure: Failure | null = null) {
    const t = this.#live(id);
    if (!t) return;
    if (stage !== "done") {
      t.closed = true;
      // A warning may stay; trust may not
      if (t.identity?.state === "trusted") t.identity = null;
    }
    // Some of these quote the peer
    t.failure = failure && new Failure(failure.code, failure.message.slice(0, MAX_WHY));
    this.#closing = t.abort;
    this.#enter(t, stage);
  }

  /** Final, and its connection is closed now. */
  kill(id: string, stage: "declined" | "cancelled" | "failed", failure: Failure | null = null) {
    if (!this.isLive(id)) return;
    this.end(id, stage, failure);
    this.#closing?.();
    this.#closing = null;
  }

  /** Call when this question is actually on screen. */
  displayed(id: string, stage: Stage) {
    const t = this.#live(id);
    if (!t || t.stage !== stage || !ASKS.includes(stage) || t.seen !== null) return;
    t.seen = this.#now();
    this.#changed();
  }

  /** The checks every decision shares. */
  #question(id: string): Current {
    const t = this.#live(id);
    if (!t) throw new Refused("not-current");
    if (!t.question || !ASKS.includes(t.stage)) throw new Refused("wrong-stage");
    if (t.seen === null || this.#now() - t.seen < ARMING_MS) throw new Refused("not-yet");
    return t;
  }

  /** Runs `decide`, one at a time per question. */
  async #deciding<T>(t: Current, decide: () => Promise<T>): Promise<T> {
    if (t.deciding) throw new Refused("wrong-stage");
    t.deciding = true;
    try {
      return await decide();
    } finally {
      t.deciding = false;
    }
  }

  /** The user compared the code and names the device. Saves the trust. */
  async verify(id: string, name: string): Promise<void> {
    const t = this.#question(id);
    if (t.stage !== "verification") throw new Refused("wrong-stage");
    const peer = t.proof;
    if (!(peer instanceof Authenticated)) throw new Refused("not-eligible");
    const question = t.question;
    await this.#deciding(t, async () => {
      try {
        await this.#devices.pin(
          peer,
          name,
          t.peer.alias,
          // Exactly what was displayed, and still the same question
          (now) => this.#live(id) === t && t.question === question && sameKind(t.identity, now),
          (trusted) => {
            if (this.#live(id) !== t) return;
            t.identity = identityOf(trusted);
            this.#open(t, true);
            this.#pass(t);
          },
        );
      } catch (e) {
        if (!(e instanceof TrustError)) throw e;
        if (e.code === "not-eligible") throw new Refused(this.isLive(id) ? "not-eligible" : "not-current");
        throw new Refused("trust", e);
      }
    });
    void this.#devices.refundAttempt(id);
  }

  /** It is the trusted device, whatever it calls itself today. This transfer only. */
  async confirmName(id: string): Promise<void> {
    const t = this.#question(id);
    if (t.stage !== "name-check") throw new Refused("wrong-stage");
    const { proof, identity } = t;
    if (!(proof instanceof Authenticated) || identity?.state !== "trusted") throw new Refused("not-eligible");
    await this.#deciding(t, async () => {
      const now = await this.#devices.standing(proof.fingerprint, t.peer.alias);
      if (this.#question(id).stage !== "name-check") throw new Refused("wrong-stage");
      if (now.kind !== "trusted" || now.name !== identity.name) throw new Refused("not-eligible");
      this.#open(t, true);
      this.#pass(t);
    });
    void this.#devices.refundAttempt(id);
  }

  /**
   * true: accept, at approval and from a trusted device only.
   * false: refuse whatever is being asked.
   */
  async respond(id: string, accept: boolean): Promise<void> {
    const t = this.#question(id);
    const peer = t.proof;
    if (!accept) {
      const trusted = t.identity?.state === "trusted";
      this.#open(t, false);
      // At approval the connection still carries the DECLINED
      if (t.stage === "approval") this.end(id, "declined");
      else this.kill(id, "declined");
      if (trusted) void this.#devices.refundAttempt(id);
      return;
    }
    if (t.stage !== "approval") throw new Refused("wrong-stage");
    if (!(peer instanceof Authenticated)) throw new Refused("not-eligible");
    await this.#deciding(t, async () => {
      const now = await this.#devices.standing(peer.fingerprint, t.peer.alias);
      if (this.#question(id).stage !== "approval") throw new Refused("wrong-stage");
      if (now.kind !== "trusted") throw new Refused("not-eligible");
      this.#open(t, true);
      this.#enter(t, "transferring");
    });
    void this.#devices.refundAttempt(id);
  }

  /** An unknown stock peer only, and never remembered. */
  async allowOnce(id: string): Promise<void> {
    const t = this.#question(id);
    const { stage, proof } = t;
    if (stage !== "unverified" && stage !== "approval") throw new Refused("wrong-stage");
    if (t.capability !== "stock" || t.identity?.state !== "new" || !(proof instanceof Connected)) {
      throw new Refused("not-eligible");
    }
    await this.#deciding(t, async () => {
      const now = await this.#devices.standing(proof.fingerprint, t.peer.alias);
      if (this.#question(id).stage !== stage) throw new Refused("wrong-stage");
      if (now.kind !== "new") throw new Refused("not-eligible");
      this.#open(t, true);
      if (stage === "approval") this.#enter(t, "transferring");
      else this.#pass(t);
    });
  }

  cancel(id: string) {
    if (!this.isLive(id)) throw new Refused("not-current");
    this.kill(id, "cancelled");
  }
}
