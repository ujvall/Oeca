/**
 * Oeca Nearby: where an incoming file goes while it arrives.
 *
 * Preferred: the origin-private file system (OPFS) through a worker, so
 * memory stays at about one batch (1 MiB) whatever the file size; the
 * finished file is a disk-backed File the user then saves (a tap, which
 * every browser accepts as the gesture a download needs). The ceiling is the
 * site's storage quota, checked before accepting.
 *
 * Fallback (no OPFS sync access, e.g. older Safari or some private modes):
 * chunks in memory, capped at MEMORY_LIMIT, because a Blob that large can
 * crash a phone's tab.
 */

import { randomSecret } from "~/services/trust";

export const MAX_FILES = 100;
export const MEMORY_LIMIT = 256 * 1024 * 1024;
const BATCH = 1024 * 1024;
const BATCH_CHUNKS = 1024;

export type Received = { name: string; size: number; file: File };

export interface Sink {
  write(chunk: ArrayBuffer): Promise<void>;
  /** The complete file (after the declared size was checked by the caller). */
  finish(): Promise<File>;
  /** Drops what was written (cancel, failure): nothing partial is kept. */
  abort(): Promise<void>;
}

let worker: Worker | null = null;
let seq = 0;
const pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();

/*
 * oeca: each tab writes in its own folder (oeca-incoming/s-<random>) and holds
 * a Web Lock named after it while it lives, so no other tab deletes what it is
 * receiving or has not saved yet. A closed or crashed tab's lock is released:
 * its folder is then removed by the next tab that starts.
 */
const OWN = /^s-[A-Za-z0-9_-]{43}$/;
const lockName = (dir: string) => `oeca-nearby-incoming:${dir}`;
let session: Promise<string> | null = null;

function ownDir(): Promise<string> {
  session ??= (async () => {
    const dir = `s-${randomSecret()}`;
    const locks = navigator.locks;
    if (locks) {
      await new Promise<void>((held) => {
        void locks.request(lockName(dir), () => {
          held();
          return new Promise<never>(() => {}); // held until the tab goes
        });
      });
    }
    return dir;
  })();
  return session;
}

async function call(op: string, extra: Record<string, unknown> = {}, transfer: Transferable[] = []): Promise<any> {
  const dir = (extra.dir as string | undefined) ?? (await ownDir());
  if (!worker) {
    worker = new Worker(new URL("../workers/sink.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (e) => {
      const p = pending.get(e.data.seq);
      pending.delete(e.data.seq);
      if (e.data.ok) p?.resolve(e.data);
      else p?.reject(new Error(e.data.error));
    };
  }
  const id = ++seq;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    worker!.postMessage({ seq: id, op, ...extra, dir }, transfer);
  });
}

let opfs: Promise<boolean> | null = null;

/** Whether streaming to OPFS works here (probed once: open, write, remove). */
export function streamingSupported(): Promise<boolean> {
  opfs ??= (async () => {
    if (typeof Worker === "undefined" || !navigator.storage?.getDirectory) return false;
    try {
      await call("open", { name: ".probe" });
      await call("remove", { name: ".probe" });
      return true;
    } catch {
      return false;
    }
  })();
  return opfs;
}

/** Bytes this site may still store (OPFS), or the memory cap. */
export async function receiveCapacity(): Promise<{ bytes: number; streaming: boolean }> {
  if (await streamingSupported()) {
    const { quota = 0, usage = 0 } = (await navigator.storage.estimate?.()) ?? {};
    // A margin: the quota is shared with the site's other storage
    return { bytes: Math.max(0, (quota - usage) * 0.9), streaming: true };
  }
  return { bytes: MEMORY_LIMIT, streaming: false };
}

/** This tab's received files that were not saved ("Done"): gone. Other tabs' are not touched. */
export async function clearOwn() {
  if (await streamingSupported()) await call("clear").catch(() => {});
}

/**
 * The folders of tabs that are gone (closed, or crashed mid-transfer). A tab
 * still open holds its folder's lock: its folder is kept. Without Web Locks
 * nothing of another tab's is touched (leftovers stay until site data is
 * cleared): no tab's files are ever at risk.
 */
export async function sweepClosedTabs() {
  if (!(await streamingSupported()) || !navigator.locks) return;
  const own = await ownDir();
  const { dirs = [] } = await call("list").catch(() => ({}));
  for (const dir of dirs as string[]) {
    if (dir === own || !OWN.test(dir)) continue;
    await navigator.locks
      .request(lockName(dir), { ifAvailable: true }, async (lock) => {
        if (lock) await call("removeDir", { dir }).catch(() => {});
      })
      .catch(() => {});
  }
}

export async function openSink(id: string, name: string): Promise<Sink> {
  const key = `${Date.now()}-${id}`;
  if (await streamingSupported()) {
    await call("open", { name: key });
    let batch: ArrayBuffer[] = [];
    let batched = 0;
    const flush = async () => {
      if (!batch.length) return;
      const chunks = batch;
      batch = [];
      batched = 0;
      await call("write", { chunks }, chunks);
    };
    return {
      async write(chunk) {
        batch.push(chunk);
        batched += chunk.byteLength;
        if (batched >= BATCH || batch.length >= BATCH_CHUNKS) await flush();
      },
      async finish() {
        await flush();
        await call("close", { name: key });
        const root = await (await navigator.storage.getDirectory()).getDirectoryHandle("oeca-incoming");
        const dir = await root.getDirectoryHandle(await ownDir());
        const f = await (await dir.getFileHandle(key)).getFile();
        return new File([f], name, { type: "application/octet-stream" });
      },
      async abort() {
        batch = [];
        await call("remove", { name: key }).catch(() => {});
      },
    };
  }
  let chunks: ArrayBuffer[] = [];
  let parts: Blob[] = [];
  let size = 0;
  return {
    async write(chunk) {
      size += chunk.byteLength;
      if (size > MEMORY_LIMIT) throw new Error("This browser keeps incoming files in memory: too large");
      chunks.push(chunk);
      if (chunks.length >= BATCH_CHUNKS) {
        parts.push(new Blob(chunks));
        chunks = [];
      }
    },
    async finish() {
      const f = new File([...parts, ...chunks], name, { type: "application/octet-stream" });
      chunks = [];
      parts = [];
      return f;
    },
    async abort() {
      chunks = [];
      parts = [];
    },
  };
}

/** Hands a received file to the browser's download (call from a tap). */
export function saveFile(file: File) {
  const url = URL.createObjectURL(file);
  const a = document.createElement("a");
  a.href = url;
  a.download = file.name;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  // The download keeps its own reference; revoking later frees the URL only
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
