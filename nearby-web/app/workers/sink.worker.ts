/// <reference lib="webworker" />
// Oeca Nearby: writes one incoming file into the origin-private file system
// (OPFS) with a synchronous access handle, so the file streams to disk
// instead of growing a Blob in memory. Sync access handles exist only in
// workers; they are the OPFS writer with the widest support (Chrome 102,
// Safari 15.2, Firefox 111).

let handle: FileSystemSyncAccessHandle | null = null;
let offset = 0;

// oeca: every op names its tab's folder (fileSink.ts); "list" and "removeDir"
// see the others, for the sweep of closed tabs' folders
async function root() {
  const opfs = await navigator.storage.getDirectory();
  return opfs.getDirectoryHandle("oeca-incoming", { create: true });
}

async function dir(name: string) {
  return (await root()).getDirectoryHandle(name, { create: true });
}

self.onmessage = async (e: MessageEvent) => {
  const { seq, op, name, chunks, dir: folder } = e.data;
  let dirs: string[] | undefined;
  try {
    if (op === "open") {
      const file = await (await dir(folder)).getFileHandle(name, { create: true });
      handle = await (file as any).createSyncAccessHandle();
      handle!.truncate(0);
      offset = 0;
    } else if (op === "write") {
      for (const c of chunks as ArrayBuffer[]) {
        offset += handle!.write(new Uint8Array(c), { at: offset });
      }
    } else if (op === "close") {
      handle?.flush();
      handle?.close();
      handle = null;
    } else if (op === "remove") {
      handle?.close();
      handle = null;
      await (await dir(folder)).removeEntry(name).catch(() => {});
    } else if (op === "clear") {
      const d = await dir(folder);
      for await (const key of (d as any).keys()) {
        await d.removeEntry(key).catch(() => {});
      }
    } else if (op === "list") {
      dirs = [];
      for await (const [key, entry] of (await root()) as any) {
        if (entry.kind === "directory") dirs.push(key);
      }
    } else if (op === "removeDir") {
      await (await root()).removeEntry(folder, { recursive: true });
    }
    postMessage({ seq, ok: true, offset, dirs });
  } catch (err) {
    postMessage({ seq, ok: false, error: String(err) });
  }
};
