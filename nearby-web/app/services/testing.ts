/**
 * Oeca Nearby: what the tests share. Synthetic identities, and just enough of
 * IndexedDB (one transaction at a time, atomic, abortable) and WebRTC to test
 * without a browser or a network. Imported by *.test.ts only.
 */
import { Devices } from "./devices";
import { connected, readSdp, respondToExchange, type Authenticated, type Connected } from "./proof";
import { commitOf, OSV, randomSecret } from "./trust";

export function fingerprint(seed: number): string {
  return `sha-256 ${Array.from({ length: 32 }, (_, i) => ((seed >> (8 * (i % 3))) & 0xff).toString(16).padStart(2, "0")).join(":")}`;
}

export function sdpOf(fp: string, lines = ""): string {
  return `v=0\r\na=fingerprint:${fp}\r\n${lines}m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n`;
}

/** A connection that applied `remote`; `der` is the certificate it exposes, if it does. */
export function fakePc(remote: string, der?: ArrayBuffer | null): RTCPeerConnection {
  const transport = der === undefined ? {} : { getRemoteCertificates: () => (der ? [der] : []) };
  return {
    localDescription: { sdp: sdpOf(fingerprint(0)) },
    remoteDescription: { sdp: sdpOf(remote) },
    sctp: { transport },
  } as unknown as RTCPeerConnection;
}

export class FakeChannel extends EventTarget {
  readyState = "open";
  sent: any[] = [];
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onSend: (message: any) => void = () => {};
  send(raw: string) {
    this.sent.push(JSON.parse(raw));
    this.onSend(JSON.parse(raw));
  }
  push(message: unknown) {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
}

export function claim(fp: string) {
  return readSdp(sdpOf(fp), OSV.COMMIT).fingerprint!;
}

/** An identity the one honest way: through the exchange. */
export async function authenticated(seed: number): Promise<Authenticated> {
  const secret = randomSecret();
  const channel = new FakeChannel();
  channel.onSend = (m) => {
    if (m.ready) queueMicrotask(() => channel.push({ r: secret, peer: "peer" }));
  };
  const fp = fingerprint(seed);
  return respondToExchange(
    fakePc(fp),
    Promise.resolve(channel as unknown as RTCDataChannel),
    { myId: "me", peerId: "peer", claimed: claim(fp) },
    await commitOf(secret),
    randomSecret(),
  );
}

export function plainConnection(seed: number): Promise<Connected> {
  return connected(fakePc(fingerprint(seed)), claim(fingerprint(seed)));
}

/** What the fake browser is made to fail at. */
export type Faults = {
  open?: boolean;
  write?: boolean;
  /** localStorage: "full" still takes an empty value. */
  storage?: "full" | "dead";
};

/** One origin: every `devices()` is another tab, or the page reloaded. */
export class FakeBrowser {
  faults: Faults = {};
  clock = 2_000_000_000_000;
  data = new Map<string, unknown>();
  kept = new Map<string, string>();
  #queue: Promise<void> = Promise.resolve();

  storage = {
    getItem: (key: string) => this.kept.get(key) ?? null,
    setItem: (key: string, value: string) => {
      const { storage } = this.faults;
      if (storage === "dead" || (storage === "full" && value !== "")) throw new Error("QuotaExceededError");
      this.kept.set(key, value);
    },
    removeItem: (key: string) => void this.kept.delete(key),
  } as unknown as Storage;

  indexedDB = {
    open: () => {
      const request: any = { result: this.#database() };
      setTimeout(() => (this.faults.open ? request.onerror?.() : request.onsuccess?.()), 0);
      return request;
    },
  } as unknown as IDBFactory;

  #database() {
    return {
      close() {},
      createObjectStore() {},
      transaction: (_store: string, mode: string) => {
        const tx: any = { error: null };
        const writes = new Map<string, unknown>();
        const deletes = new Set<string>();
        let aborted = false;
        const reads: (() => void)[] = [];
        const settled: (() => void)[] = [];
        const settle = () => {
          const request: any = {};
          settled.push(() => request.onsuccess?.());
          return request;
        };
        tx.abort = () => (aborted = true);
        tx.objectStore = () => ({
          get: (key: string) => {
            const request: any = {};
            reads.push(() => {
              request.result = structuredClone(this.data.get(key));
              request.onsuccess?.();
            });
            return request;
          },
          put: (value: unknown, key: string) => (writes.set(key, structuredClone(value)), settle()),
          delete: (key: string) => (deletes.add(key), settle()),
        });
        this.#queue = this.#queue.then(async () => {
          await new Promise((resolve) => setTimeout(resolve, 0));
          for (const read of reads) read();
          if (!aborted && (writes.size || deletes.size) && (this.faults.write || mode !== "readwrite")) {
            tx.error = { name: "QuotaExceededError" };
            aborted = true;
          }
          if (aborted) return void tx.onabort?.();
          for (const [key, value] of writes) this.data.set(key, value);
          for (const key of deletes) this.data.delete(key);
          for (const done of settled) done();
          tx.oncomplete?.();
        });
        return tx;
      },
    };
  }

  devices(): Devices {
    return new Devices({ indexedDB: () => this.indexedDB, storage: () => this.storage, now: () => this.clock });
  }
}
