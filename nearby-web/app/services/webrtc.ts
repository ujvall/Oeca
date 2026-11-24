// Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md.
import {
  SignalingConnection,
  type WsServerSdpMessage,
} from "~/services/signaling";
import { decodeBase64, encodeBase64 } from "~/utils/base64";
import { StreamController } from "~/utils/streamController";
import {
  chunkStream,
  LOW_BUFFERED_AMOUNT,
  MAX_BUFFERED_AMOUNT,
  waitBufferDrained,
} from "~/utils/dataChannel";
import pako from "pako";
import { commitOf, OSV, randomSecret, withSessionLines } from "~/services/trust";
import {
  type Authenticated,
  type Capability,
  type ClaimedFingerprint,
  type Connected,
  connected,
  type Exchange,
  Failure,
  initiate,
  readSdp,
  respondToExchange,
  usable,
} from "~/services/proof";
import type { Received, Sink } from "~/utils/fileSink";
import { generateNonce, validateNonce } from "~/utils/nonce";
import { generateClientTokenFromNonce } from "~/services/crypto";
import {
  Approved,
  CONTROL_BACKLOG,
  inflateSdp,
  MAX_CHANNELS,
  MAX_CONTROL_BYTES,
  MAX_CONTROL_CHUNKS,
  MAX_SIGNALED_CHARS,
  MAX_TOKEN_CHARS,
  MESSAGE_COST,
  parseFileList,
  TRANSFER_BACKLOG,
} from "~/services/limits";

export const protocolVersion = "2.3";

// oeca: LocalSend's own STUN server, as the native peer uses
export const defaultStun = ["stun:stun.localsend.org:3478"];

/** oeca: how long the other side may take to answer, and to connect. */
const ANSWER_TIMEOUT_MS = 30_000;
const CONNECT_TIMEOUT_MS = 30_000;

/** oeca: how a session ended without an exception. */
export type SessionResult = "done" | "declined";

/**
 * oeca: rejects when the connection fails or closes, or the user cancels, so
 * no step waits forever on a peer that is gone. Reads from the data channel
 * stream reject too (the stream is errored).
 */
function guard(
  pc: RTCPeerConnection,
  signal: AbortSignal | undefined,
  stream: () => StreamController<string | ArrayBuffer> | null,
) {
  let fail!: (e: Error) => void;
  const failed = new Promise<never>((_, reject) => (fail = reject));
  failed.catch(() => {});
  // A lost connection ends the stream gracefully: messages already received
  // (a DECLINED sent just before the peer closed) are still read. A cancel
  // discards them.
  let channel: RTCDataChannel | null = null;
  const end = (why: string | Error, discard: boolean) => {
    const e = typeof why === "string" ? new Error(why) : why;
    if (discard) stream()?.error(e);
    else stream()?.close();
    fail(e);
    // A cancel also stops whatever is mid-flight (the chunk loop); the data
    // channel's close reaches the peer at once (its session ends on it)
    if (discard) void closeGracefully(channel, pc);
  };
  pc.addEventListener("connectionstatechange", () => {
    if (["failed", "closed", "disconnected"].includes(pc.connectionState)) end("Connection lost", false);
  });
  signal?.addEventListener("abort", () => end("Cancelled", true));
  if (signal?.aborted) end("Cancelled", true);
  /** The session's data channel: its close by the peer (a cancel there) ends this side too. */
  const watchChannel = (dc: RTCDataChannel) => {
    channel = dc;
    dc.addEventListener("close", () => end("Stopped by the other device", false));
  };
  const race = <T>(p: Promise<T>, ms?: number, what = "Timed out"): Promise<T> =>
    Promise.race([
      p,
      failed,
      ...(ms ? [new Promise<never>((_, r) => setTimeout(() => r(new Error(what)), ms))] : []),
    ]);
  return { race, end, watchChannel };
}

/** oeca: what a session needs from the transfer's state machine (transfer.ts). */
export type Decisions = {
  /** Resolves once this connection may go on; rejects if it may not. */
  admit(proof: Connected | Authenticated): Promise<void>;
  /** Rejects if trust was withdrawn meanwhile. */
  allowed(): Promise<void>;
};

/** oeca: whatever went wrong during authentication is an authentication failure. */
function authenticationFailed(e: unknown): never {
  if (e instanceof Failure) throw e;
  throw new Failure("authentication", e instanceof Error ? e.message : "Verification did not complete");
}

/**
 * oeca: closes the data channel first and waits (briefly) for its close to
 * complete: the SCTP stream reset is LocalSend-compatible and tells the peer
 * now; closing the peer connection at once could drop it unsent.
 */
async function closeGracefully(dc: RTCDataChannel | null, pc: RTCPeerConnection) {
  if (dc && dc.readyState !== "closed") {
    const closed = new Promise<void>((resolve) => dc.addEventListener("close", () => resolve(), { once: true }));
    dc.close();
    await Promise.race([closed, new Promise((r) => setTimeout(r, 500))]);
  }
  pc.close();
}

export async function sendFiles({
  signaling,
  stunServers,
  fileDtoList,
  fileMap,
  targetId,
  signingKey,
  onFilesSkip,
  onFileProgress,
  certificate,
  signal,
  myId,
  onClaim,
  decisions,
}: {
  signaling: SignalingConnection;
  stunServers: string[];
  fileDtoList: FileDto[];
  fileMap: Record<string, File>;
  targetId: string;
  signingKey: CryptoKeyPair;
  onFilesSkip: (fileIds: string[]) => void;
  onFileProgress: (progress: FileProgress) => void;
  // oeca: persistent DTLS identity, cancel, and the verification gate that
  // runs before any file name or content leaves this device
  certificate?: RTCCertificate;
  signal?: AbortSignal;
  myId: string;
  onClaim: (fingerprint: ClaimedFingerprint | null, capability: Capability) => void;
  decisions: Decisions;
}): Promise<SessionResult> {

  const peerConnection = await createPeerConnection(stunServers, certificate);

  const dataChannel = peerConnection.createDataChannel("data");
  dataChannel.binaryType = "arraybuffer";
  dataChannel.bufferedAmountLowThreshold = LOW_BUFFERED_AMOUNT;
  // oeca: a receiver only ever answers: its unread replies stay small
  const dataChannelStream = createStreamController(
    dataChannel,
    () => CONTROL_BACKLOG,
    (e) => end(e, true),
  );
  const dataChannelOpened = new Promise<void>((resolve) => {
    dataChannel.onopen = () => resolve();
  });
  const { race, end, watchChannel } = guard(peerConnection, signal, () => dataChannelStream);
  watchChannel(dataChannel);
  const countChannel = channelLimit((e) => end(e, true));
  peerConnection.ondatachannel = () => countChannel();
  try {


  const offer = await peerConnection.createOffer();
  await peerConnection.setLocalDescription(offer);
  await race(waitICEGathering(peerConnection), CONNECT_TIMEOUT_MS, "Could not prepare the connection");
  const localSdp = peerConnection.localDescription!.sdp;


  // oeca: unpredictable (another room peer must not guess it)
  const sessionId = randomSecret();

  // oeca: commit to a secret before seeing the peer's random (see trust.ts)
  const secret = randomSecret();
  const commitLine = `a=${OSV.COMMIT}:${await commitOf(secret)}`;
  stopIfOver(signal);
  signaling.send({
    type: "OFFER",
    sessionId: sessionId,
    target: targetId,
    sdp: encodeSdp(withSessionLines(localSdp, [commitLine])),
  });


  const answer = await race(signaling.waitForAnswer(sessionId, targetId), ANSWER_TIMEOUT_MS, "No answer from the other device");
  const answerSdp = decodeSdp(answer.sdp);


  // oeca: claims only, until the connection proves them
  const said = readSdp(answerSdp, OSV.NONCE);
  onClaim(said.fingerprint, said.capability);
  if (said.busy) throw new Failure("busy");
  const { claimed, value: answerNonce } = usable(said);

  await peerConnection.setRemoteDescription({
    type: "answer",
    sdp: answerSdp,
  });

  // oeca: an Oeca peer answered with its random: authenticate it on the
  // extension channel, alongside LocalSend's handshake. Never opened to others.
  let authenticated: Promise<Authenticated> | null = null;
  if (answerNonce) {
    const exchange: Exchange = { myId, peerId: targetId, claimed };
    authenticated = race(
      initiate(peerConnection, exchange, secret, answerNonce),
      CONNECT_TIMEOUT_MS,
      "Verification did not complete in time",
    ).catch(authenticationFailed);
    authenticated.catch((e: Error) => end(e, true));
  }

  await race(dataChannelOpened, CONNECT_TIMEOUT_MS, "Could not connect to the other device");


  const localNonce = await generateNonce();
  dataChannel.send(
    JSON.stringify({
      nonce: encodeBase64(localNonce),
    } as RTCNonceMessage),
  );
  const remoteNonce = await receiveNonce(dataChannelStream);
  const nonce = new Uint8Array(localNonce.length + remoteNonce.length);
  nonce.set(localNonce);
  nonce.set(remoteNonce, localNonce.length);


  const localToken = await generateClientTokenFromNonce(signingKey, nonce);
  dataChannel.send(
    JSON.stringify({
      token: localToken,
    } as RTCTokenRequest),
  );

  const tokenResponseRaw = await dataChannelStream.readNext();
  if (typeof tokenResponseRaw !== "string") {
    throw new Error("Expected string");
  }
  const tokenResponse = JSON.parse(tokenResponseRaw) as RTCTokenResponse;
  if (tokenResponse.status === "INVALID_SIGNATURE") {
    console.error("Invalid signature");
    throw new Error("The other device rejected the signature");
  }

  // oeca: no PIN, either way. Any peer could ask for one, and what the user
  // typed would go to that peer: a request for one ends the transfer, unanswered
  if (tokenResponse.status === "PIN_REQUIRED") throw new Error(PIN_REFUSED);
  if (tokenResponse.status !== "OK") {
    console.error("Invalid response");
    throw new Error("Unexpected response from the other device");
  }
  const remoteToken = tokenResponse.token;


  // oeca: authenticated, and decided, before the file list (names, sizes) is sent
  const proof = await race<Authenticated | Connected>(authenticated ?? connected(peerConnection, claimed));
  await race(decisions.admit(proof));
  await race(decisions.allowed());
  stopIfOver(signal);

  sendStringInChunks(
    dataChannel,
    JSON.stringify({
      status: "OK",
      files: fileDtoList,
    } as RTCFileListMessage),
  );

  sendDelimiter(dataChannel);


  let fileListResponseRaw = await receiveStringFromChunks(dataChannelStream);
  let fileListResponse = JSON.parse(fileListResponseRaw) as RTCFileListResponse;
  let fileTokens: Record<string, string>;
  if (fileListResponse.status === "OK") {
    fileTokens = fileListResponse.files;
  } else if (fileListResponse.status === "PAIR") {
    dataChannel.send(
      JSON.stringify({
        status: "PAIR_DECLINED",
      } as RTCPairResponse),
    );
    fileListResponseRaw = await receiveStringFromChunks(dataChannelStream);
    fileListResponse = JSON.parse(fileListResponseRaw) as RTCFileListResponse;
    if (fileListResponse.status === "OK") {
      fileTokens = fileListResponse.files;
    } else {
      return declined(fileListResponse);
    }
  } else {
    return declined(fileListResponse);
  }

  // oeca: only a token of our own files' ids, as a string
  const given = Object(fileTokens);
  fileTokens = Object.create(null);
  for (const { id } of fileDtoList) {
    const token = Object.hasOwn(given, id) ? given[id] : undefined;
    if (typeof token === "string" && token && token.length <= MAX_TOKEN_CHARS) fileTokens[id] = token;
  }

  const skippedFiles: string[] = [];
  fileDtoList = fileDtoList.filter((file) => {
    const hasToken = fileTokens[file.id];
    if (!hasToken) {
      skippedFiles.push(file.id);
    }
    return hasToken;
  });

  if (skippedFiles.length > 0) {
    onFilesSkip(skippedFiles);
  }
  if (fileDtoList.length === 0) return "declined";

  let unsaved = skippedFiles.length;
  await race(decisions.allowed());

  const firstFileDto = fileDtoList[0];
  if (!firstFileDto) return "declined";
  dataChannel.send(
    JSON.stringify({
      id: firstFileDto.id,
      token: fileTokens[firstFileDto.id],
    } as RTCSendFileHeaderRequest),
  );

  for (const [i, fileDto] of fileDtoList.entries()) {
    // oeca: the list is a subset of our own files (filtered above)
    const file = fileMap[fileDto.id];
    if (!file) throw new Failure("other", "A file to send is missing.");
    const fileSize = file.size;
    await sendFileInChunks(dataChannel, file, (bytes) => {
      onFileProgress({
        id: fileDto.id,
        curr: bytes,
      });
    });

    const nextFileDto = fileDtoList[i + 1];
    if (nextFileDto) {
      await race(decisions.allowed());
      const fileToken = fileTokens[nextFileDto.id];

      dataChannel.send(
        JSON.stringify({
          id: nextFileDto.id,
          token: fileToken,
        } as RTCSendFileHeaderRequest),
      );
    } else {
      sendDelimiter(dataChannel);
    }

    const fileStatus = await dataChannelStream.readNext();
    if (typeof fileStatus !== "string") {
      throw new Error("Expected string");
    }

    const response = Object(JSON.parse(fileStatus)) as RTCSendFileResponse;
    // oeca: saved is what the receiver confirmed, for exactly this file
    if (response.id !== fileDto.id || typeof response.success !== "boolean") {
      throw new Failure("other", "The other device's confirmation does not match the file that was sent.");
    }
    if (!response.success) unsaved++;
    onFileProgress({
      id: fileDto.id,
      curr: fileSize,
      success: response.success,
      error: response.success ? undefined : String(response.error || "Not saved").slice(0, 300),
    });
  }

  if (unsaved) {
    const all = fileDtoList.length + skippedFiles.length;
    throw new Failure("other", `${unsaved} of ${all} files were not saved by the other device.`);
  }
  return "done";
  } finally {
    // oeca: every way out closes the connection, so the peer learns at once
    await closeGracefully(dataChannel, peerConnection);
  }
}

export async function receiveFiles({
  signaling,
  stunServers,
  offer,
  signingKey,
  selectFiles,
  onFileProgress,
  certificate,
  signal,
  openSink,
  onReceived,
  offerSdp,
  identity,
}: {
  signaling: SignalingConnection;
  stunServers: string[];
  offer: WsServerSdpMessage;
  signingKey: CryptoKeyPair;
  // oeca: the user's decision, with the connection's identity to verify
  selectFiles: (files: FileDto[]) => Promise<string[]>;
  onFileProgress: (progress: FileProgress) => void;
  certificate?: RTCCertificate;
  signal?: AbortSignal;
  openSink: (file: FileDto) => Promise<Sink>;
  onReceived: (file: Received) => void;
  /** oeca: the offer's SDP, as the caller read it. */
  offerSdp: string;
  /**
   * oeca: what the offer claimed, and whom to ask. null only to decline a
   * stock peer while busy: then nothing is ever selected.
   */
  identity: {
    claimed: ClaimedFingerprint;
    /** An Oeca offer, already counted against the attempt limit. */
    verification: { myId: string; commit: string } | null;
    decisions: Decisions;
  } | null;
}): Promise<SessionResult> {

  const peerConnection = await createPeerConnection(stunServers, certificate);
  let dataChannelStream: StreamController<string | ArrayBuffer> | null = null;
  let dataChannel: RTCDataChannel | null = null;
  let sink: Sink | null = null;
  const { race, end, watchChannel } = guard(peerConnection, signal, () => dataChannelStream);
  const countChannel = channelLimit((e) => end(e, true));
  let backlog = CONTROL_BACKLOG;
  try {

  // oeca: LocalSend's channel by its label ("data"); the extension's apart
  let extChannel!: (c: RTCDataChannel) => void;
  const extChannelPromise = new Promise<RTCDataChannel>((resolve) => (extChannel = resolve));
  const dataChannelPromise = new Promise<RTCDataChannel>((resolve) => {
    peerConnection.ondatachannel = (event) => {
      if (!countChannel()) return;
      if (event.channel.label === OSV.CHANNEL) extChannel(event.channel);
      else if (event.channel.label === "data") resolve(event.channel);
    };
  });

  // oeca: an Oeca offer carries a commitment: answer with our random
  const answerNonce = identity?.verification ? randomSecret() : null;

  await peerConnection.setRemoteDescription({
    type: "offer",
    sdp: offerSdp,
  });

  const answer = await peerConnection.createAnswer();
  await peerConnection.setLocalDescription(answer);
  await race(waitICEGathering(peerConnection), CONNECT_TIMEOUT_MS, "Could not prepare the connection");
  const localSdp = peerConnection.localDescription!.sdp;


  stopIfOver(signal);
  signaling.send({
    type: "ANSWER",
    sessionId: offer.sessionId,
    target: offer.peer.id,
    sdp: encodeSdp(withSessionLines(localSdp, answerNonce ? [`a=${OSV.NONCE}:${answerNonce}`] : [])),
  });


  dataChannel = await race(dataChannelPromise, CONNECT_TIMEOUT_MS, "Could not connect to the other device");
  dataChannel.binaryType = "arraybuffer";


  dataChannelStream = createStreamController(
    dataChannel,
    () => backlog,
    (e) => end(e, true),
  );
  watchChannel(dataChannel);
  const channel = dataChannel;
  const stream = dataChannelStream;

  if (channel.readyState !== "open") {
    await race(
      new Promise<void>((resolve) => {
        channel.onopen = () => resolve();
      }),
      CONNECT_TIMEOUT_MS,
      "Could not connect to the other device",
    );
  }

  // oeca: who this connection is, before any list. The user's decision runs
  // beside the handshake: the sender's screen needs it to show the same code.
  let decided: Promise<void> | null = null;
  if (identity) {
    const { claimed, verification, decisions } = identity;
    const proof =
      verification && answerNonce
        ? await race(
            respondToExchange(
              peerConnection,
              extChannelPromise,
              { myId: verification.myId, peerId: offer.peer.id, claimed },
              verification.commit,
              answerNonce,
            ),
            CONNECT_TIMEOUT_MS,
            "Verification failed: the other device did not reveal its secret",
          ).catch(authenticationFailed)
        : await race(connected(peerConnection, claimed));
    decided = decisions.admit(proof);
    decided.catch((e: Error) => end(e, true));
  }


  const remoteNonce = await receiveNonce(stream);
  const localNonce = await generateNonce();
  channel.send(
    JSON.stringify({
      nonce: encodeBase64(localNonce),
    } as RTCNonceMessage),
  );
  const nonce = new Uint8Array(localNonce.length + remoteNonce.length);
  nonce.set(remoteNonce);
  nonce.set(localNonce, remoteNonce.length);

  const remoteTokenRaw = await stream.readNext();
  if (typeof remoteTokenRaw !== "string") {
    throw new Error("Expected string");
  }

  const remoteToken = JSON.parse(remoteTokenRaw) as RTCTokenRequest;

  const localToken = await generateClientTokenFromNonce(signingKey, nonce);
  // oeca: this side never asks for a PIN (see sendFiles)
  channel.send(
    JSON.stringify({
      status: "OK",
      token: localToken,
    } as RTCTokenResponse),
  );

  const listMessage = JSON.parse(await receiveStringFromChunks(stream)) as RTCFileListMessage;
  if (listMessage.status === "PIN_REQUIRED") throw new Error(PIN_REFUSED);
  if (listMessage.status !== "OK") throw new Error("Unexpected response from the other device");

  const fileList = parseFileList(listMessage.files);

  if (decided) await race(decided);
  // oeca: without an identity there is only the decline
  const selectedFiles = identity ? await race(selectFiles(fileList)) : [];

  // oeca: nothing selected is a decline, said in the protocol (as the native
  // core does), instead of an empty OK the sender cannot tell from a success
  if (selectedFiles.length === 0) {
    sendStringInChunks(channel, JSON.stringify({ status: "DECLINED" } as RTCFileListResponse));
    sendDelimiter(channel);
    await waitBufferEmpty(channel);
    return "declined";
  }

  const approved = new Approved<FileDto>();
  // oeca: ids are the sender's: never keys of an ordinary object
  const selectedFilesTokens: Record<string, string> = Object.create(null);
  for (const file of fileList) {
    if (selectedFiles.includes(file.id)) {
      const token = randomSecret();
      approved.add(file, token);
      selectedFilesTokens[file.id] = token;
    }
  }
  backlog = TRANSFER_BACKLOG;


  sendStringInChunks(
    channel,
    JSON.stringify({
      status: "OK",
      files: selectedFilesTokens,
    } as RTCFileListResponse),
  );

  sendDelimiter(channel);


  const dataChannelIterator = stream.createAsyncIterator();
  // oeca: chunks go to a sink (disk when the browser allows), not a Blob in memory
  let fileState: { id: string; curr: number; file: FileDto } | null = null;
  let complete = false;
  for await (const chunk of dataChannelIterator.asyncIterator) {
    if (typeof chunk === "string") {
      if (fileState) {
        const dto = fileState.file;
        if (fileState.curr !== dto.size) {
          throw new Error(`${dto.fileName}: incomplete`);
        }
        onReceived({ name: dto.fileName, size: dto.size, file: await sink!.finish() });
        sink = null;

        onFileProgress({
          id: fileState.id,
          curr: fileState.curr,
          success: true,
        });

        // Send status of last file
        channel.send(
          JSON.stringify({
            id: fileState.id,
            success: true,
          } as RTCSendFileResponse),
        );

        if (chunk.length <= 1) {
          // End of all files
          // Wait for the last status to be sent
          fileState = null;
          complete = true;
          await waitBufferEmpty(channel);
          break;
        }
      }

      const header = Object(JSON.parse(chunk)) as RTCSendFileHeaderRequest;
      // oeca: only files we selected, with the token we gave for them, each once
      const file = approved.take(header.id, header.token);
      fileState = {
        id: file.id,
        curr: 0,
        file,
      };
      await race(identity!.decisions.allowed());
      sink = await openSink(file);
    } else {
      if (!fileState || !sink) {
        throw new Error("Expected file state");
      }
      fileState.curr += chunk.byteLength;
      if (fileState.curr > fileState.file.size) {
        throw new Error("More data than declared");
      }
      await sink.write(chunk);
      onFileProgress({
        id: fileState.id,
        curr: fileState.curr,
      });
    }
  }
  dataChannelIterator.releaseLock();
  // oeca: the stream ended (connection closed) before the last delimiter
  if (!complete) {
    throw new Error(channel.readyState === "closed" ? "Stopped by the other device" : "Connection lost");
  }
  if (approved.remaining) {
    throw new Failure("other", `The other device ended without sending ${approved.remaining} of the files.`);
  }
  return "done";
  } finally {
    // oeca: a partial file is dropped, and the connection always closed
    await sink?.abort();
    await closeGracefully(dataChannel, peerConnection);
  }
}

async function createPeerConnection(
  stunServers: string[],
  certificate?: RTCCertificate,
): Promise<RTCPeerConnection> {
  const peerConnection = new RTCPeerConnection({
    certificates: certificate ? [certificate] : undefined,
    iceServers:
      stunServers.length === 0
        ? undefined
        : [
            {
              urls: stunServers,
            },
          ],
  });

  // oeca: the event carries addresses and server URLs: only its code is logged
  peerConnection.onicecandidateerror = (event) => {
    console.warn("ICE candidate error", (event as RTCPeerConnectionIceErrorEvent).errorCode);
  };

  return peerConnection;
}

/** oeca: counts the channels a peer opens; false once there are too many. */
function channelLimit(end: (e: Error) => void) {
  let opened = 0;
  return () => {
    if (++opened <= MAX_CHANNELS) return true;
    if (opened === MAX_CHANNELS + 1) end(new Failure("other", "The other device opened too many channels"));
    return false;
  };
}

function createStreamController(
  dataChannel: RTCDataChannel,
  limit: () => number,
  overflow: (e: Error) => void,
) {
  const dataChannelStream = new StreamController<string | ArrayBuffer>({
    cost: (m) => (typeof m === "string" ? m.length : m.byteLength) + MESSAGE_COST,
    limit,
    overflow,
  });
  dataChannel.onmessage = (event) => {
    dataChannelStream.add(event.data);
  };
  return dataChannelStream;
}

/** Oeca Hop asks for no PIN and enters none: any peer can ask for one. */
export const PIN_REFUSED = "The other device asks for a PIN. Oeca Hop does not use PINs: turn the PIN off on that device.";

export type FileProgress = {
  id: string;
  curr: number;
  success?: boolean;
  error?: string;
};

export type FileDto = {
  id: string;
  fileName: string;
  size: number;
  fileType: string;
  sha256?: string;
  preview?: string;
  metadata?: FileMetadata;
};

export type FileMetadata = {
  modified?: string;
  accessed?: string;
};

type RTCNonceMessage = {
  nonce: string;
};

type RTCTokenRequest = {
  token: string;
};

type RTCTokenResponse =
  | {
      status: "OK";
      token: string;
    }
  | {
      // A stock peer's: refused (PIN_REFUSED), never answered
      status: "PIN_REQUIRED";
    }
  | {
      status: "INVALID_SIGNATURE";
    };

/** The sender's file list. A stock sender may ask for a PIN instead: refused. */
type RTCFileListMessage =
  | {
      status: "OK";
      files: FileDto[];
    }
  | {
      status: "PIN_REQUIRED";
    };

type RTCFileListResponse =
  | {
      status: "OK";
      files: Record<string, string>;
    }
  | {
      status: "PAIR";
      publicKey: string;
    }
  | {
      status: "DECLINED";
    }
  | {
      status: "INVALID_SIGNATURE";
    };

type RTCPairResponse =
  | {
      status: "OK";
      publicKey: string;
    }
  | {
      status: "PAIR_DECLINED";
    }
  | {
      status: "INVALID_SIGNATURE";
    };

type RTCSendFileHeaderRequest = {
  id: string;
  token: string;
};

type RTCSendFileResponse = {
  id: string;
  success: boolean;
  error?: string;
};

/** oeca: only DECLINED is the other user's decline. Anything else is a failure. */
function declined(response: RTCFileListResponse): "declined" {
  if (response.status === "DECLINED") return "declined";
  throw new Failure("other", "The other device did not take the files.");
}

/** oeca: a cancelled or withdrawn session signals and sends nothing more. */
function stopIfOver(signal?: AbortSignal) {
  if (signal?.aborted) throw new Error("Cancelled");
}

export function encodeSdp(s: string): string {
  const data = new TextEncoder().encode(s);
  const compressed = pako.deflate(data);
  return encodeBase64(compressed);
}

export function decodeSdp(s: string): string {
  // oeca: bounded before and while inflating (zlib expands about 1000x)
  if (s.length > MAX_SIGNALED_CHARS) throw new Error("SDP too large");
  return inflateSdp(decodeBase64(s));
}

async function receiveNonce(
  dataChannelStream: StreamController<string | ArrayBuffer>,
): Promise<Uint8Array> {
  const remoteNonce = await dataChannelStream.readNext();
  if (typeof remoteNonce !== "string") {
    throw new Error("Expected string");
  }

  const nonceMsg = JSON.parse(remoteNonce) as RTCNonceMessage;
  const decodedNonce = decodeBase64(nonceMsg.nonce);

  if (!validateNonce(decodedNonce)) {
    throw new Error("Invalid remote nonce");
  }

  return decodedNonce;
}


async function receiveStringFromChunks(
  dataChannelStream: StreamController<string | ArrayBuffer>,
): Promise<string> {
  let dataChannelIterator = dataChannelStream.createAsyncIterator();
  let chunks: ArrayBuffer[] = [];
  let delimited = false;
  let size = 0;
  for await (const chunk of dataChannelIterator.asyncIterator) {
    if (typeof chunk === "string") {
      delimited = true;
      break;
    }
    // oeca: bounded while it accumulates, not once it is whole
    size += chunk.byteLength;
    if (size > MAX_CONTROL_BYTES || chunks.length >= MAX_CONTROL_CHUNKS) {
      throw new Failure("other", "The other device's message is too large");
    }
    chunks.push(chunk);
  }
  dataChannelIterator.releaseLock();
  // oeca: the connection ended mid-message
  if (!delimited) throw new Error("Connection lost");

  return arrayBufferToString(chunks);
}

function sendDelimiter(dataChannel: RTCDataChannel) {
  dataChannel.send("0");
}

const CHUNK_SIZE = 16 * 1024; // 16 KiB

function sendStringInChunks(dataChannel: RTCDataChannel, str: string) {
  const utf8Binary = new TextEncoder().encode(str);
  for (let i = 0; i < utf8Binary.length; i += CHUNK_SIZE) {
    dataChannel.send(utf8Binary.slice(i, i + CHUNK_SIZE));
  }
}

/**
 * Send a file in chunks, pausing whenever the send queue is full.
 * @param dataChannel
 * @param file
 * @param onProgress
 */
async function sendFileInChunks(
  dataChannel: RTCDataChannel,
  file: File,
  onProgress: (bytes: number) => void,
) {
  let bytesSent = 0;

  for await (const chunk of chunkStream(file, CHUNK_SIZE)) {
    if (dataChannel.bufferedAmount > MAX_BUFFERED_AMOUNT) {
      await waitBufferDrained(dataChannel);
    }

    dataChannel.send(chunk);

    bytesSent += chunk.length;
    onProgress(bytesSent);
  }
}

function arrayBufferToString(arrayBuffers: ArrayBuffer[]): string {
  const totalLength = arrayBuffers.reduce(
    (sum, buffer) => sum + buffer.byteLength,
    0,
  );
  const combinedArray = new Uint8Array(totalLength);
  let offset = 0;
  arrayBuffers.forEach((buffer) => {
    combinedArray.set(new Uint8Array(buffer), offset);
    offset += buffer.byteLength;
  });
  return new TextDecoder().decode(combinedArray);
}

async function waitBufferEmpty(dataChannel: RTCDataChannel) {
  // oeca: a peer that reads nothing cannot keep the session; a closed queue never drains
  const until = performance.now() + 5000;
  while (dataChannel.bufferedAmount > 0 && dataChannel.readyState === "open" && performance.now() < until) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}

async function waitICEGathering(localConnection: RTCPeerConnection) {
  if (localConnection.iceGatheringState === "complete") {
    return;
  }

  await new Promise<void>((resolve) => {
    localConnection.onicegatheringstatechange = () => {
      if (localConnection.iceGatheringState === "complete") {
        resolve();
      }
    };
  });
}
