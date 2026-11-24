/**
 * Oeca Nearby: the trusted-device store (the same rules as Oeca's
 * src/nearby/devices.rs). It needs no connection: the device list works offline.
 *
 * - A record is one certificate fingerprint, under the name this user
 *   confirmed. What the device called itself is kept apart, only to notice a
 *   change; no record is found, replaced or removed by a name.
 * - Forgetting keeps the record, as revoked, and never drops one to make room.
 * - Every write is one IndexedDB transaction (so tabs cannot lose each other's
 *   updates) and counts only once it completed.
 * - A revocation is effective before it is written. A hold in localStorage
 *   covers it until it is saved, also across a reload; while there is one, all
 *   trust is paused. `held: false` means not even that could be written.
 * - A store that cannot be read trusts nobody and is never written over.
 */
import { canonicalFingerprint } from "./trust";
import { Authenticated } from "./proof";

const DB = "oeca-nearby-trust";
const STORE = "state";
const KEY = "records";
const HOLD = "oeca-nearby-trust-hold";
const OPEN_TIMEOUT_MS = 5000;

export const MAX_TRUSTED = 256;
/** Trusted and revoked together. Forgetting never adds a record, so it always fits. */
export const MAX_RECORDS = 4096;

/**
 * Per 10 minutes, saved, counted before anything the sender claims is looked at.
 * ponytail: a peer that reconnects for new room ids can still use up the total
 * (nothing unforgeable identifies it before authentication); blocks guessing, not denial
 */
export const MAX_ATTEMPTS = 10;
export const MAX_ATTEMPTS_PER_PEER = 4;
export const ATTEMPT_WINDOW_MS = 10 * 60 * 1000;
const MAX_NAME = 64;

export type DeviceState = "trusted" | "revoked" | "revocation-pending";

/** What the device list shows. */
export type Device = {
  /** The name this user confirmed. */
  name: string;
  fingerprint: string;
  short: string;
  /** Milliseconds since 1970. */
  verifiedAt: number;
  revokedAt: number | null;
  /** "revocation-pending": distrusted now, not saved yet (`retry`). */
  state: DeviceState;
};

export type Status = "ready" | "unavailable" | "unreadable" | "paused";

/** What the store says about a key, with the name it came under. */
export type Standing =
  | { kind: "new" }
  /** conflict: another name than the confirmed one, or another trusted device's. */
  | { kind: "trusted"; name: string; conflict: boolean }
  /** An unknown key under a known (trusted or revoked) device's name. */
  | { kind: "changed" }
  | { kind: "revoked" }
  /** The store cannot answer: nobody is trusted. */
  | { kind: "unavailable" };

export type TrustErrorCode =
  | "unavailable"
  | "unreadable"
  | "paused"
  | "not-found"
  | "invalid-name"
  | "name-in-use"
  | "full"
  /** pin: the question changed or ended meanwhile. */
  | "not-eligible"
  /** pin: not trusted. */
  | "write-failed"
  /** forget: distrusted now, not saved. */
  | "not-saved";

export class TrustError extends Error {
  constructor(
    readonly code: TrustErrorCode,
    /** not-saved: a reload keeps trust paused. */
    readonly held = false,
    readonly reason = "",
  ) {
    super(reason ? `${code}: ${reason}` : code);
  }
}

type Row = { key: string; name: string; alias: string; verified: number; revoked: number | null };
type Attempt = { at: number; peer: string; transfer: string };
type Records = { version: 1; devices: Row[]; attempts: Attempt[] };
type Pending = { all: boolean; keys: string[]; unknown: boolean };

export type Change = {
  /** Withdrawn this moment, before any write. */
  revoked?: { all: boolean; keys: string[] };
};

export function shortFingerprint(fingerprint: string): string {
  return (
    fingerprint
      .slice(8)
      .replaceAll(":", "")
      .slice(0, 16)
      .toUpperCase()
      .match(/.{4}/g) ?? []
  ).join(" ");
}

/** A name fit to show and to compare: one line, no controls or invisible marks. */
export function cleanName(name: string): string {
  // A peer's alias can be as large as a message
  const shown = name
    .slice(0, MAX_NAME * 8)
    .replace(/[­؜᠎​-‏‪-‮⁠-⁯﻿]|[\u{e0000}-\u{e007f}]/gu, "")
    .replace(/[\s\u0085]+/gu, " ")
    .replace(/\p{Cc}/gu, "")
    .replace(/ +/g, " ")
    .trim();
  return [...shown].slice(0, MAX_NAME).join("").trimEnd();
}

function sameName(a: string, b: string) {
  return a !== "" && a.toLowerCase() === b.toLowerCase();
}

function isText(v: unknown): v is string {
  return typeof v === "string";
}

function isTime(v: unknown): v is number {
  return Number.isSafeInteger(v) && (v as number) >= 0;
}

/** null: there, but not a store this version knows. */
function decode(v: any): Records | null {
  if (v === undefined) return { version: 1, devices: [], attempts: [] };
  if (v?.version !== 1 || !Array.isArray(v.devices) || !Array.isArray(v.attempts)) return null;
  const seen = new Set<string>();
  for (const d of v.devices) {
    if (!isText(d?.key) || canonicalFingerprint(d.key) !== d.key || seen.has(d.key)) return null;
    if (!isText(d.name) || !isText(d.alias) || !isTime(d.verified)) return null;
    if (d.revoked !== null && !isTime(d.revoked)) return null;
    seen.add(d.key);
  }
  for (const a of v.attempts) {
    if (!isTime(a?.at) || !isText(a.peer) || !isText(a.transfer)) return null;
  }
  return v;
}

function revokes(p: Pending) {
  return p.all || p.keys.length > 0;
}

function paused(p: Pending) {
  return p.unknown || revokes(p);
}

function covers(p: Pending, key: string) {
  return p.all || p.keys.includes(key);
}

function union(a: Pending, b: Pending): Pending {
  return { all: a.all || b.all, keys: [...new Set([...a.keys, ...b.keys])], unknown: a.unknown || b.unknown };
}

const nothing = (): Pending => ({ all: false, keys: [], unknown: false });

function standingIn(records: Records | null, pending: Pending, key: string, liveAlias: string): Standing {
  const known = records?.devices.find((r) => r.key === key);
  if (covers(pending, key) || (known && known.revoked !== null)) return { kind: "revoked" };
  if (!records || paused(pending)) return { kind: "unavailable" };
  const alias = cleanName(liveAlias);
  const goesBy = (r: Row) => sameName(r.name, alias) || sameName(r.alias, alias);
  const others = records.devices.filter((r) => r.key !== key);
  if (known) {
    const conflict = !goesBy(known) || others.some((r) => r.revoked === null && goesBy(r));
    return { kind: "trusted", name: known.name, conflict };
  }
  return others.some(goesBy) ? { kind: "changed" } : { kind: "new" };
}

type Sources = {
  indexedDB?: () => IDBFactory | undefined;
  storage?: () => Storage | undefined;
  now?: () => number;
};

export class Devices {
  #pending = nothing();
  #listeners = new Set<(change: Change) => void>();
  #tabs: BroadcastChannel | null = null;
  #idb: () => IDBFactory | undefined;
  #storage: () => Storage | undefined;
  #now: () => number;

  constructor(sources: Sources = {}) {
    this.#idb = sources.indexedDB ?? (() => globalThis.indexedDB);
    this.#storage = sources.storage ?? (() => globalThis.localStorage);
    this.#now = sources.now ?? (() => Date.now());
  }

  /** Follows the other tabs. Call in the browser only. */
  listen() {
    if (this.#tabs || typeof BroadcastChannel === "undefined") return;
    this.#tabs = new BroadcastChannel(DB);
    this.#tabs.onmessage = ({ data }) => {
      // Another tab could not save it: it holds here too
      if (data?.unsaved) this.#pending = union(this.#pending, data.unsaved);
      this.#emit(data?.revoked ? { revoked: data.revoked } : {});
    };
  }

  close() {
    this.#tabs?.close();
    this.#tabs = null;
  }

  onChange(listener: (change: Change) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  #emit(change: Change) {
    for (const listener of this.#listeners) listener(change);
  }

  #changed(change: Change = {}, unsaved?: Pending) {
    this.#emit(change);
    try {
      this.#tabs?.postMessage({ ...change, unsaved });
    } catch {
      // the channel is closed: other tabs read the store at their next decision
    }
  }

  #open(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      const fail = () => {
        clearTimeout(timer);
        reject(new TrustError("unavailable"));
      };
      const timer = setTimeout(fail, OPEN_TIMEOUT_MS);
      try {
        const factory = this.#idb();
        if (!factory) return fail();
        const open = factory.open(DB, 1);
        open.onupgradeneeded = () => open.result.createObjectStore(STORE);
        open.onerror = fail;
        open.onblocked = fail;
        open.onsuccess = () => {
          clearTimeout(timer);
          open.result.onversionchange = () => open.result.close();
          resolve(open.result);
        };
      } catch {
        fail();
      }
    });
  }

  /**
   * One transaction. It resolves only when that completed; `saved` runs at
   * that moment, before anyone is told.
   */
  async #transact<T>(
    mode: IDBTransactionMode,
    change: (records: Records) => { next?: Records; out: T },
    saved?: (out: T) => void,
  ): Promise<T> {
    const db = await this.#open();
    try {
      return await new Promise<T>((resolve, reject) => {
        const tx = db.transaction(STORE, mode, { durability: "strict" });
        let why: unknown = null;
        let out: T;
        const failed = () =>
          reject(why instanceof TrustError ? why : new TrustError("write-failed", false, String((why as any)?.name ?? "abort")));
        tx.onabort = () => {
          why ??= tx.error;
          failed();
        };
        tx.oncomplete = () => {
          saved?.(out);
          resolve(out);
        };
        const store = tx.objectStore(STORE);
        const read = store.get(KEY);
        read.onsuccess = () => {
          try {
            const records = decode(read.result);
            if (!records) throw new TrustError("unreadable");
            const result = change(records);
            out = result.out;
            if (result.next) store.put(result.next, KEY);
          } catch (e) {
            why = e;
            tx.abort();
          }
        };
      });
    } finally {
      db.close();
    }
  }

  #hold(): Pending {
    let raw: string | null;
    try {
      const storage = this.#storage();
      raw = storage ? storage.getItem(HOLD) : null;
    } catch {
      return { all: false, keys: [], unknown: true };
    }
    if (raw === null) return nothing();
    try {
      const v = JSON.parse(raw);
      const valid =
        typeof v.all === "boolean" &&
        Array.isArray(v.keys) &&
        v.keys.every((k: unknown) => isText(k) && canonicalFingerprint(k) === k) &&
        (v.all || v.keys.length > 0);
      if (valid) return { all: v.all, keys: v.keys, unknown: false };
    } catch {
      // empty or damaged: it holds, without saying whom
    }
    return { all: false, keys: [], unknown: true };
  }

  #effective(): Pending {
    return union(this.#pending, this.#hold());
  }

  /** True only if what is stored then covers all that is pending. */
  #writeHold(p: Pending, replaceUnknown: boolean): boolean {
    const stored = this.#hold();
    if (!stored.unknown || replaceUnknown) {
      const all = union({ ...stored, unknown: false }, p);
      for (const text of [JSON.stringify({ all: all.all, keys: all.keys }), ""]) {
        try {
          const storage = this.#storage();
          if (!storage) break;
          storage.setItem(HOLD, text);
          return true;
        } catch {
          // full: try the empty hold, which still pauses
        }
      }
    } else {
      try {
        if (this.#storage()?.getItem(HOLD) === "") return true; // holds everything
      } catch {
        return false;
      }
    }
    const now = this.#hold();
    return now.all || (!p.all && p.keys.every((k) => now.keys.includes(k)));
  }

  /** What was saved stops being pending, and nothing else. */
  #settled(saved: Pending, replaceUnknown: boolean) {
    const left = (p: Pending): Pending => ({
      all: p.all && !saved.all,
      keys: p.keys.filter((k) => !saved.keys.includes(k)),
      unknown: p.unknown,
    });
    this.#pending = left(this.#pending);
    const stored = this.#hold();
    if (stored.unknown && !replaceUnknown) return;
    const rest = left({ ...stored, unknown: false });
    try {
      if (revokes(rest)) this.#storage()?.setItem(HOLD, JSON.stringify({ all: rest.all, keys: rest.keys }));
      else this.#storage()?.removeItem(HOLD);
    } catch {
      // left behind, it only repeats these revocations at the next load
    }
  }

  async #revoke(asked: { all: boolean; keys: string[] }): Promise<void> {
    const before = union(this.#pending, this.#hold());
    this.#pending = union(this.#pending, { ...asked, unknown: false });
    this.#changed({ revoked: asked });
    const replaceUnknown = asked.all;
    // Written ahead: a reload during the transaction keeps it revoked
    const held = this.#writeHold(this.#pending, replaceUnknown);
    const pending = this.#effective();
    try {
      await this.#transact(
        "readwrite",
        (records) => {
          if (!pending.all && asked.keys.some((k) => !records.devices.some((d) => d.key === k))) {
            throw new TrustError("not-found");
          }
          const at = this.#now();
          for (const d of records.devices) {
            if (covers(pending, d.key) && d.revoked === null) d.revoked = at;
          }
          return { next: records, out: undefined };
        },
        () => this.#settled(pending, replaceUnknown),
      );
    } catch (e) {
      if (e instanceof TrustError && e.code === "not-found") {
        // Nothing to revoke: withdraw only what this call added
        const added = asked.keys.filter((k) => !before.keys.includes(k));
        this.#settled({ all: false, keys: added, unknown: false }, false);
        this.#changed();
        throw e;
      }
      this.#changed({}, this.#pending);
      const reason = e instanceof TrustError ? e.reason || e.code : "";
      throw new TrustError("not-saved", held, reason);
    }
    this.#changed();
  }

  /**
   * Revokes one device. It is distrusted before anything is written: whatever
   * this resolves or rejects with, it is not trusted afterwards.
   */
  forget(fingerprint: string): Promise<void> {
    if (canonicalFingerprint(fingerprint) !== fingerprint) return Promise.reject(new TrustError("not-found"));
    return this.#revoke({ all: false, keys: [fingerprint] });
  }

  /** Revokes every device. Also how an unknown hold is resolved. */
  forgetAll(): Promise<void> {
    return this.#revoke({ all: true, keys: [] });
  }

  /** Saves what is waiting. Never restores trust that was withdrawn. */
  async retry(): Promise<void> {
    const pending = this.#effective();
    if (revokes(pending)) {
      const held = this.#writeHold(pending, false);
      try {
        await this.#transact(
          "readwrite",
          (records) => {
            const at = this.#now();
            for (const d of records.devices) {
              if (covers(pending, d.key) && d.revoked === null) d.revoked = at;
            }
            return { next: records, out: undefined };
          },
          () => this.#settled(pending, false),
        );
      } catch (e) {
        throw new TrustError("not-saved", held, e instanceof TrustError ? e.reason || e.code : "");
      }
      this.#changed();
    }
    const status = await this.status();
    if (status !== "ready") throw new TrustError(status);
  }

  async status(): Promise<Status> {
    try {
      await this.#transact("readonly", () => ({ out: undefined }));
    } catch (e) {
      if (paused(this.#effective())) return "paused";
      return e instanceof TrustError && e.code === "unreadable" ? "unreadable" : "unavailable";
    }
    return paused(this.#effective()) ? "paused" : "ready";
  }

  /** Trusted and revoked devices, trusted first. */
  async list(): Promise<Device[]> {
    const rows = await this.#transact("readonly", (records) => ({ out: records.devices }));
    const pending = this.#effective();
    return rows
      .map((r): Device => ({
        name: r.name,
        fingerprint: r.key,
        short: shortFingerprint(r.key),
        verifiedAt: r.verified,
        revokedAt: r.revoked,
        state: covers(pending, r.key) ? "revocation-pending" : r.revoked === null ? "trusted" : "revoked",
      }))
      .sort(
        (a, b) =>
          Number(a.state !== "trusted") - Number(b.state !== "trusted") ||
          a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
      );
  }

  async standing(fingerprint: string, liveAlias: string): Promise<Standing> {
    // Read on both sides of the wait: another tab may save and drop its hold meanwhile
    const before = this.#effective();
    let records: Records | null = null;
    try {
      records = await this.#transact("readonly", (r) => ({ out: r }));
    } catch {
      // unreadable or unavailable: nobody is trusted
    }
    if (covers(before, fingerprint)) return { kind: "revoked" };
    return standingIn(records, this.#effective(), fingerprint, liveAlias);
  }

  /**
   * Trusts an authenticated peer under the name the user confirmed. `eligible`
   * is asked inside the transaction; `saved` runs as it completes.
   */
  async pin(
    peer: Authenticated,
    name: string,
    liveAlias: string,
    eligible: (standing: Standing) => boolean = () => true,
    saved?: (standing: Standing) => void,
  ): Promise<void> {
    if (!(peer instanceof Authenticated)) throw new TrustError("not-eligible");
    const key = peer.fingerprint;
    const confirmed = cleanName(name);
    if (!confirmed) throw new TrustError("invalid-name");
    await this.#transact(
      "readwrite",
      (records) => {
        const pending = this.#effective();
        if (paused(pending)) throw new TrustError("paused");
        if (!eligible(standingIn(records, pending, key, liveAlias))) throw new TrustError("not-eligible");
        const others = records.devices.filter((r) => r.revoked === null && r.key !== key);
        if (others.some((r) => sameName(r.name, confirmed))) throw new TrustError("name-in-use");
        if (others.length >= MAX_TRUSTED) throw new TrustError("full");
        const row: Row = { key, name: confirmed, alias: cleanName(liveAlias), verified: this.#now(), revoked: null };
        const own = records.devices.findIndex((r) => r.key === key);
        if (own >= 0) records.devices[own] = row;
        else if (records.devices.length >= MAX_RECORDS) throw new TrustError("full");
        else records.devices.push(row);
        return { next: records, out: standingIn(records, pending, key, liveAlias) };
      },
      saved,
    );
    this.#changed();
  }

  /** Counts one handshake before it is answered. False: do not answer. */
  async beginAttempt(peer: string, transfer: string): Promise<boolean> {
    try {
      return await this.#transact("readwrite", (records) => {
        if (paused(this.#effective())) return { out: false };
        const at = this.#now();
        const recent = records.attempts
          .map((a) => ({ ...a, at: Math.min(a.at, at) })) // a clock set back never shortens the window
          .filter((a) => at - a.at < ATTEMPT_WINDOW_MS);
        const byPeer = recent.filter((a) => a.peer === peer).length;
        if (recent.length >= MAX_ATTEMPTS || byPeer >= MAX_ATTEMPTS_PER_PEER) return { out: false };
        records.attempts = [...recent, { at, peer, transfer }];
        return { next: records, out: true };
      });
    } catch {
      return false; // uncountable: not answered
    }
  }

  /** Only after the user decided on an authenticated, trusted peer. */
  async refundAttempt(transfer: string): Promise<void> {
    try {
      await this.#transact("readwrite", (records) => {
        const left = records.attempts.filter((a) => a.transfer !== transfer);
        if (left.length === records.attempts.length) return { out: undefined };
        records.attempts = left;
        return { next: records, out: undefined };
      });
    } catch {
      // still counted: the safe direction
    }
  }
}

export const devices = new Devices();
