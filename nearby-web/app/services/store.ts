// Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md.
import type {
  ClientInfo,
  ClientInfoWithoutId,
  WsServerMessage,
  WsServerSdpMessage,
} from "~/services/signaling";
import { SignalingConnection } from "~/services/signaling";
import {
  decodeSdp,
  type Decisions,
  defaultStun,
  encodeSdp,
  type FileDto,
  type FileProgress,
  receiveFiles,
  sendFiles,
  type SessionResult,
} from "~/services/webrtc";
import { generateClientTokenFromCurrentTimestamp } from "~/services/crypto";
import { deleteIdentity, OSV, randomSecret } from "~/services/trust";
import {
  MAX_DECLINES,
  MAX_SESSION_ID,
  MAX_SIGNALED_CHARS,
  RATE_LIMITED_WAIT_MS,
  reconnectIn,
  Refusals,
  STABLE_MS,
} from "~/services/limits";
import { Failure, readSdp, usable } from "~/services/proof";
import { devices, TrustError } from "~/services/devices";
import { Board, isFinal, Refused, type Stage, type TransferView } from "~/services/transfer";
import {
  clearOwn,
  sweepClosedTabs,
  MAX_FILES,
  openSink,
  type Received,
  receiveCapacity,
} from "~/utils/fileSink";
import { formatBytes } from "~/utils/fileSize";

export enum SessionState {
  idle = "idle",
  sending = "sending",
  receiving = "receiving",
}

/** oeca: the signaling connection, as the page header shows it. */
export type Link = "connecting" | "online" | "reconnecting";

export type FileState = {
  id: string;
  name: string;
  curr: number;
  total: number;
  state: "pending" | "skipped" | "sending" | "finished" | "error";
  error?: string;
};

/** Back after this long in the background: the socket is likely dead. */
const STALE_AFTER_HIDDEN_MS = 30_000;

/** oeca: not reset by a reconnect. */
const refusals = new Refusals();
let declines = 0;
const DECLINE_TIMEOUT_MS = 30_000;
/** oeca: a finished file counts as this much progress, so many tiny files are not a drip. */
const FILE_PROGRESS = 4096;
let wakeLock: { release(): Promise<void> } | null = null;
let wakeLoop: (() => void) | null = null;
let hiddenAt = 0;
/** oeca: the network or the tab came back since the last attempt. */
let hurried = false;

export const store = reactive({
  // Whether the connection loop has started
  _loopStarted: false,

  // Client information of the current user that we send to the server
  _proposingClient: null as ClientInfoWithoutId | null,

  // Public and private key pair for signing and verifying messages
  key: null as CryptoKeyPair | null,

  // oeca: this browser's DTLS identity (see trust.ts)
  certificate: null as RTCCertificate | null,
  persistentIdentity: false,

  // Signaling connection to the server
  signaling: null as SignalingConnection | null,
  link: "connecting" as Link,
  /** oeca: the server answered 429; the next attempt waits its 15 minutes. */
  limited: false,

  // Client information of the current user that we received from the server
  client: null as ClientInfo | null,

  // oeca: from the build's configuration (setupConnection)
  stunServers: defaultStun as string[],

  // List of peers connected to the same room
  peers: [] as ClientInfo[],

  // Current session information
  session: {
    state: SessionState.idle,
    /** oeca: the transfer: its id, stage and who it is. A copy (see transfer.ts). */
    transfer: null as TransferView | null,
    /** The file picker is open for this peer. */
    selecting: null as { id: string; alias: string } | null,
    message: "",
    /** Received files, kept (on disk when streaming) until saved or dismissed. */
    received: [] as Received[],
    /** The tab went to the background during this transfer. */
    wasHidden: false,
    curr: 0,
    total: 1, // Avoid division by zero
    fileState: {} as Record<string, FileState>,
  },
});

/** oeca: every decision goes through it, by transfer id (transfer.ts). */
export const board = new Board(devices, (view) => {
  store.session.transfer = view;
  if (!view || isFinal(view.stage)) void holdWake(false);
});

export async function setupConnection({
  url,
  info,
  stunServers,
}: {
  url: string;
  info: ClientInfoWithoutId;
  stunServers?: string[];
}) {
  store._proposingClient = info;
  if (stunServers) store.stunServers = stunServers;
  if (!store._loopStarted) {
    store._loopStarted = true;
    watchLifecycle();
    void sweepClosedTabs();
    devices.listen();
    // A revocation a closed tab could not finish
    void devices.retry().catch(() => {});
    void connectionLoop(url);
  }
}

async function connectionLoop(url: string) {
  let delay = 1000;
  let limitedUntil = 0;
  while (true) {
    const started = performance.now();
    hurried = false;
    try {
      store.signaling = await SignalingConnection.connect({
        url: url,
        info: store._proposingClient!,
        onMessage: (data: WsServerMessage) => {
          switch (data.type) {
            case "HELLO":
              store.client = data.client;
              store.peers = data.peers;
              store.link = "online";
              store.limited = false;
              break;
            case "ERROR":
              if (data.code === 429) {
                limitedUntil = performance.now() + RATE_LIMITED_WAIT_MS;
                store.limited = true;
                store.signaling?.close();
              }
              break;
            case "JOIN":
              store.peers = [...store.peers.filter((p) => p.id !== data.peer.id), data.peer];
              break;
            case "UPDATE":
              store.peers = store.peers.map((p) =>
                p.id === data.peer.id ? data.peer : p,
              );
              break;
            case "LEFT":
              store.peers = store.peers.filter((p) => p.id !== data.peerId);
              break;
            case "OFFER":
              acceptOffer({ offer: data });
              break;
            case "ANSWER":
              break;
          }
        },
        generateNewInfo: async () => {
          const token = await generateClientTokenFromCurrentTimestamp(
            store.key!,
          );
          updateClientTokenState(token);
          return { ...store._proposingClient!, token };
        },
        onClose: () => {
          store.signaling = null;
          store.client = null;
          store.peers = [];
          store.link = "reconnecting";
          // oeca: an offer of the room that is gone can get no answer
          const t = store.session.transfer;
          if (t?.stage === "signaling") {
            board.kill(t.id, "failed", new Failure("other", "Connection to the signaling server lost"));
          }
        },
      });

      await store.signaling.waitUntilClose();
    } catch (error) {
      store.link = store.link === "connecting" ? "connecting" : "reconnecting";
    }
    // oeca: backoff, reset only by a connection that lasted; cut short when the
    // network or the tab comes back, but never below the server's own wait
    const ended = performance.now();
    if (ended - started >= STABLE_MS) delay = 1000;
    const until = () => {
      const now = performance.now();
      return now + reconnectIn({ now, started, delay, hurried, limitedUntil });
    };
    let at = until();
    await new Promise<void>((resolve) => {
      let timer: ReturnType<typeof setTimeout>;
      const arm = () => {
        timer = setTimeout(resolve, Math.max(0, at - performance.now()));
      };
      arm();
      wakeLoop = () => {
        const sooner = until();
        if (sooner >= at) return;
        at = sooner;
        clearTimeout(timer);
        arm();
      };
    });
    wakeLoop = null;
    delay = Math.min(delay * 2, 30_000);
  }
}

/** oeca: browser lifecycle: background tabs, sleep, network changes. */
function watchLifecycle() {
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      hiddenAt = Date.now();
      if (busy()) store.session.wasHidden = true;
      return;
    }
    if (wakeLock === null && busy()) void holdWake(true);
    // A tab that slept: its socket may look open and be dead. Not during a
    // transfer (that runs on its own connection, with its own timeouts).
    if (Date.now() - hiddenAt > STALE_AFTER_HIDDEN_MS && !busy()) reconnectNow();
  });
  window.addEventListener("online", () => reconnectNow());
}

/** oeca: the page's Retry: as when the network comes back (never sooner than the server's wait). */
export function retryNow() {
  reconnectNow();
}

function reconnectNow() {
  hurried = true;
  if (store.signaling) store.signaling.close();
  else wakeLoop?.();
}

function busy() {
  return board.busy() || store.session.selecting !== null;
}

/** Keeps the phone's screen on during a transfer (where supported). */
async function holdWake(on: boolean) {
  try {
    if (on && !wakeLock && "wakeLock" in navigator) {
      wakeLock = await (navigator as any).wakeLock.request("screen");
    } else if (!on && wakeLock) {
      await wakeLock.release();
      wakeLock = null;
    }
  } catch {
    wakeLock = null;
  }
}

export function updateAliasState(alias: string) {
  store._proposingClient!.alias = alias;
  store.client!.alias = alias;
}

function updateClientTokenState(token: string) {
  store._proposingClient!.token = token;
  store.client!.token = token;
}

/*
 * oeca: the decisions. Each names the transfer it was made for, and rejects
 * with Refused if that is not what is being asked (transfer.ts).
 */

/** Call when a question is on screen: answers count ARMING_MS later. */
export function displayed(id: string, stage: Stage) {
  board.displayed(id, stage);
}

/** The codes match, and this is what the user calls the device. */
export function verifyPeer(id: string, name: string) {
  return board.verify(id, name);
}

/** It is the trusted device, whatever it calls itself today. */
export function confirmName(id: string) {
  return board.confirmName(id);
}

/** true: accept at approval. false: refuse what is being asked. */
export function respond(id: string, accept: boolean) {
  return board.respond(id, accept);
}

/** An unknown stock LocalSend peer, this one transfer. */
export function allowOnce(id: string) {
  return board.allowOnce(id);
}

/**
 * oeca: a new identity for this browser from the next visit (reload the page
 * after). Devices that trusted it see a new device and must verify it again;
 * they cannot be told. The devices trusted here are kept (they did not
 * change), unless `forgetDevices`: then each is revoked, as "Forget" does.
 * Refused while a transfer runs.
 */
export async function resetIdentity(forgetDevices: boolean): Promise<void> {
  if (busy()) throw new Refused("busy");
  if (forgetDevices) await devices.forgetAll();
  await deleteIdentity();
}

export function cancelSession(id: string) {
  if (board.isLive(id)) board.cancel(id);
}

/** Withdraws trust in that finished transfer's device. Rejects if it is not saved. */
export async function forgetPeer(id: string) {
  const t = board.view();
  if (t?.id !== id || t.connection.state !== "authenticated") throw new Refused("not-current");
  await devices.forget(t.connection.fingerprint);
}

/** What to tell the user about a refused decision. */
export function explain(e: unknown): string {
  const trust = e instanceof TrustError ? e : e instanceof Refused ? e.trust : undefined;
  if (trust) {
    switch (trust.code) {
      case "invalid-name":
        return "Give this device a name first.";
      case "name-in-use":
        return "Another trusted device already has that name. Choose another.";
      case "full":
        return "The list of trusted devices is full. Nothing was saved.";
      case "not-saved":
        return trust.held
          ? "Trust is withdrawn, but not saved yet: no device is trusted until it is."
          : "Trust is withdrawn only until this page closes: it could not be saved. Try again.";
      default:
        return "The list of trusted devices could not be saved. Nothing was trusted.";
    }
  }
  if (e instanceof Refused && e.code === "not-yet") return "Check the screen, then tap again.";
  if (e instanceof Refused && e.code === "peer-changed") return "That device changed or left. Select it again.";
  return "That no longer applies.";
}

function explainFailure(code: string, message: string): string {
  switch (code) {
    case "busy":
      return "The other device is busy. Try again shortly.";
    case "too-many-attempts":
      return "Too many verification attempts: wait 10 minutes (possible interception).";
    case "downgraded":
      return "A device you know connected without Oeca's verification. Refused (possible interception).";
    case "revoked":
      return "Stopped: trust in this device was withdrawn or has changed.";
    case "trust-unavailable":
      return "The list of trusted devices cannot be used, so nothing was trusted.";
    case "authentication":
      return `This device could not be authenticated. ${message}`;
    default:
      return message;
  }
}

export function failureText(t: TransferView): string {
  return t.failure ? explainFailure(t.failure.code, t.failure.message) : "";
}

/** Back to the device list; received files not saved are dropped. */
export function dismissSession() {
  if (busy()) return;
  board.clear();
  store.session.state = SessionState.idle;
  store.session.message = "";
  store.session.received = [];
  void clearOwn();
}

export function markSelecting(peer: { id: string; alias: string }) {
  // oeca: a stranger who has proven nothing does not block this user's send
  const squatter = board.unprovenIncoming();
  if (squatter && !store.session.selecting) board.kill(squatter, "cancelled");
  if (busy()) return false;
  board.clear();
  resetSession();
  store.session.selecting = { id: peer.id, alias: peer.alias };
  return true;
}

export function selectionCancelled() {
  store.session.selecting = null;
}

function resetSession() {
  store.session.message = "";
  store.session.received = [];
  store.session.wasHidden = false;
  store.session.curr = 0;
  store.session.total = 1;
  store.session.fileState = Object.create(null);
}

function decisionsFor(id: string): Decisions {
  return {
    admit: async (proof) => {
      const gate = await board.admit(id, proof);
      if (gate && !(await gate.answer)) throw new Failure("other", "Not confirmed");
    },
    allowed: () => board.assertAllowed(id),
  };
}

/** How a session ended. A transfer that already ended stays as it ended. */
function ended(id: string, result: SessionResult | Error) {
  const t = board.view();
  if (!t || !board.isLive(id)) return;
  const proven = t.connection.state !== "claimed";
  if (result === "done") {
    board.end(id, "done");
  } else if (result instanceof Failure) {
    board.end(id, "failed", result);
  } else if (!proven && t.capability === "oeca") {
    const why = result === "declined" ? "The connection ended before it was authenticated" : result.message;
    board.end(id, "failed", new Failure("authentication", why));
  } else if (result === "declined") {
    if (proven) board.end(id, "declined");
    else board.end(id, "failed", new Failure("other", "The connection ended"));
  } else if (proven && result.message === "Stopped by the other device") {
    board.end(id, "cancelled", new Failure("other", "The other device stopped the transfer."));
  } else if (["Connection lost", "No more data"].includes(result.message)) {
    const message = store.session.wasHidden
      ? "The connection ended while this page was in the background. Keep it open during transfers."
      : "The connection to the other device ended.";
    board.end(id, "failed", new Failure("other", message));
  } else {
    board.end(id, "failed", new Failure("other", result.message || "Something went wrong."));
  }
}

/**
 * Offers `files` to `peer`, exactly as it was displayed when selected. Rejects
 * (Refused) if it is no longer that.
 */
export async function startSendSession({
  files,
  peer,
}: {
  /** oeca: the picker's FileList, or the files the user kept from it. */
  files: ArrayLike<File>;
  peer: { id: string; alias: string };
}): Promise<void> {
  store.session.selecting = null;
  if (board.busy()) throw new Refused("busy");
  const { signaling, client } = store;
  if (!signaling || !client) throw new Refused("offline");
  if (!store.peers.some((p) => p.id === peer.id && p.alias === peer.alias)) throw new Refused("peer-changed");

  const abort = new AbortController();
  resetSession();
  store.session.state = SessionState.sending;
  const id = board.begin("send", peer, () => abort.abort());
  const fileState: Record<string, FileState> = Object.create(null);

  const fileDtoList = convertFileListToDto(files);
  const byId = Object.fromEntries(Array.from(files, (f, i) => [i.toString(), f]));
  const fileMap = fileDtoList.reduce(
    (acc, file) => {
      const f = byId[file.id];
      if (f) acc[file.id] = f;
      fileState[file.id] = {
        id: file.id,
        name: file.fileName,
        curr: 0,
        total: file.size,
        state: "pending",
      };
      return acc;
    },
    {} as Record<string, File>,
  );

  store.session.fileState = fileState;
  store.session.total = fileDtoList.reduce((acc, file) => acc + file.size, 0) || 1;
  void holdWake(true);

  try {
    ended(
      id,
      await sendFiles({
        signaling: signaling as SignalingConnection,
        stunServers: store.stunServers,
        fileDtoList: fileDtoList,
        fileMap: fileMap,
        targetId: peer.id,
        signingKey: store.key!,
        onFilesSkip: (fileIds) => {
          if (!board.isLive(id)) return;
          for (const fileId of fileIds) {
            const f = store.session.fileState[fileId];
            if (f) f.state = "skipped";
          }
        },
        onFileProgress: (p) => {
          board.sending(id);
          onFileProgress(id, p);
        },
        certificate: store.certificate ? toRaw(store.certificate) : undefined,
        signal: abort.signal,
        myId: client.id,
        onClaim: (fingerprint, capability) => board.claim(id, fingerprint, capability),
        decisions: decisionsFor(id),
      }),
    );
  } catch (e) {
    ended(id, e as Error);
  }
}

function convertFileListToDto(files: ArrayLike<File>): FileDto[] {
  const result: FileDto[] = [];
  for (const [i, file] of Array.from(files).entries()) {
    result.push({
      id: i.toString(),
      fileName: file.name,
      size: file.size,
      fileType: file.type,
      // oeca: no dates: when a file was made or changed is the sender's business
    });
  }

  return result;
}

/** An Oeca peer that cannot be answered now is told so: no nonce, no connection, still Oeca. */
function refuseBusy(signaling: SignalingConnection, offer: WsServerSdpMessage) {
  signaling.send({
    type: "ANSWER",
    sessionId: offer.sessionId,
    target: offer.peer.id,
    sdp: encodeSdp(`v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\na=${OSV.BUSY}:1\r\n`),
  });
}

export async function acceptOffer({
  offer,
}: {
  offer: WsServerSdpMessage;
}) {
  const signaling = store.signaling as SignalingConnection | null;
  const myId = store.client?.id;
  if (!signaling) return;
  if (typeof offer.sdp !== "string" || offer.sdp.length > MAX_SIGNALED_CHARS) return;
  if (typeof offer.sessionId !== "string" || offer.sessionId.length > MAX_SESSION_ID) return;
  if (typeof offer.peer?.id !== "string" || typeof offer.peer.alias !== "string") return;
  let offerSdp = "";
  try {
    offerSdp = decodeSdp(offer.sdp);
  } catch {
    // unreadable: it claims nothing usable
  }
  const said = readSdp(offerSdp, OSV.COMMIT);
  const oeca = said.capability === "oeca";
  const common = {
    signaling,
    stunServers: store.stunServers,
    offer: offer,
    offerSdp,
    signingKey: store.key!,
    certificate: store.certificate ? toRaw(store.certificate) : undefined,
    // Stored under a name of ours: the id is the sender's
    openSink: (f: FileDto) => openSink(randomSecret(), f.fileName),
  };

  // oeca: one session at a time. Oeca hears "busy"; a stock peer a decline
  // Received files not saved yet count: taking an offer would drop them
  if (busy() || store.session.received.length > 0) {
    try {
      usable(said);
    } catch {
      return; // nothing that could be answered
    }
    if (oeca) {
      if (refusals.allow(offer.peer.id)) refuseBusy(signaling, offer);
    } else if (declines < MAX_DECLINES && refusals.allow(offer.peer.id)) {
      declines++;
      const declining = new AbortController();
      const timer = setTimeout(() => declining.abort(), DECLINE_TIMEOUT_MS);
      void receiveFiles({
        ...common,
        identity: null,
        signal: declining.signal,
        selectFiles: async () => [],
        onFileProgress: () => {},
        onReceived: () => {},
      })
        .catch(() => {})
        .finally(() => {
          clearTimeout(timer);
          declines--;
        });
    }
    return;
  }

  const abort = new AbortController();
  resetSession();
  store.session.state = SessionState.receiving;
  const id = board.begin("receive", offer.peer, () => abort.abort());
  board.claim(id, said.fingerprint, said.capability);
  const decisions = decisionsFor(id);

  try {
    const { claimed, value: commit } = usable(said);
    let verification: { myId: string; commit: string } | null = null;
    if (commit) {
      if (!myId) throw new Failure("authentication", "This device's room id is not known yet");
      // Our answer lets the sender compute the code before it reveals, so it
      // is counted first, whoever the offer says it is from
      if (!(await devices.beginAttempt(offer.peer.id, id))) {
        const limited = (await devices.status()) === "ready";
        if (refusals.allow(offer.peer.id)) refuseBusy(signaling, offer);
        throw new Failure(limited ? "too-many-attempts" : "trust-unavailable");
      }
      if (!board.isLive(id)) return;
      verification = { myId, commit };
    }
    ended(
      id,
      await receiveFiles({
        ...common,
        identity: { claimed, verification, decisions },
        signal: abort.signal,
        selectFiles: async (files) => {
          await decisions.allowed();
          // Limits first: never ask for something this browser cannot hold
          const { bytes, streaming } = await receiveCapacity();
          if (!board.isLive(id)) return [];
          const total = files.reduce((acc, file) => acc + file.size, 0) || 1;
          let refusal = "";
          if (files.length > MAX_FILES) {
            refusal = `Declined: more than ${MAX_FILES} files.`;
          } else if (total > bytes) {
            refusal = streaming
              ? `Declined: ${formatBytes(total)} is more than this browser lets the page store (${formatBytes(bytes)}).`
              : `Declined: this browser keeps incoming files in memory, up to ${formatBytes(bytes)}.`;
          }
          if (refusal) {
            board.end(id, "declined", new Failure("other", refusal));
            return [];
          }
          store.session.total = total;
          store.session.fileState = Object.create(null);
          for (const file of files) {
            store.session.fileState[file.id] = {
              id: file.id,
              name: file.fileName,
              curr: 0,
              total: file.size,
              state: "pending",
            };
          }
          const gate = board.offer(id, files);
          if (!gate || !(await gate.answer)) return [];
          await decisions.allowed();
          void holdWake(true);
          return files.map((file) => file.id);
        },
        onFileProgress: (p) => onFileProgress(id, p),
        onReceived: (file) => {
          if (board.isLive(id)) store.session.received.push(markRaw(file));
        },
      }),
    );
  } catch (e) {
    ended(id, e as Error);
  }
}

function onFileProgress(id: string, progress: FileProgress) {
  if (!board.isLive(id)) return;
  const file = store.session.fileState[progress.id];
  if (!file) return;
  file.curr = progress.curr;
  store.session.curr = Object.values(store.session.fileState).reduce(
    (acc, file) => acc + file.curr,
    0,
  );
  if (progress.success) {
    file.state = "finished";
  } else if (progress.error) {
    file.state = "error";
    file.error = progress.error;
  }
  const finished = Object.values(store.session.fileState).filter((f) => f.state === "finished").length;
  board.progress(id, store.session.curr + finished * FILE_PROGRESS);
}
