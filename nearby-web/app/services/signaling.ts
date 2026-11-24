// Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md.
import { encodeStringToBase64 } from "~/utils/base64";
import { MAX_SIGNALING_MESSAGE } from "~/services/limits";

export class SignalingConnection {
  private _socket: WebSocket;
  private _onAnswer: OnAnswer | null = null;

  private constructor(socket: WebSocket) {
    this._socket = socket;
  }

  /**
   * Connects to the signaling server.
   * @param url The URL of the signaling server.
   * @param info The client info to send to the server.
   * @param onMessage The callback to call when a message is received.
   * @param generateNewInfo The function to generate and publish a new info.
   * @param onClose The callback to call when the connection is closed.
   */
  public static async connect({
    url,
    info,
    onMessage,
    generateNewInfo,
    onClose,
  }: {
    url: string;
    info: ClientInfoWithoutId;
    onMessage: OnMessageCallback;
    generateNewInfo: () => Promise<ClientInfoWithoutId>;
    onClose: () => void;
  }): Promise<SignalingConnection> {

    const encodedInfo = encodeStringToBase64(JSON.stringify(info));
    const socket = await new Promise<WebSocket>((resolve, reject) => {
      const ws = new WebSocket(`${url}?d=${encodedInfo}`);
      ws.onopen = () => resolve(ws);
      ws.onerror = (err) => reject(err);
    });

    // ping every 120 seconds to keep the connection alive
    const pingInterval = setInterval(() => {
      socket.send("");
    }, 120 * 1000);

    // every half hour, generate a new fingerprint
    const fingerprintInterval = setInterval(
      async () => {
        const info = await generateNewInfo();
        socket.send(
          JSON.stringify({
            type: "UPDATE",
            info: info,
          } as WsClientUpdateMessage),
        );
      },
      30 * 60 * 1000,
    );

    socket.onclose = () => {
      clearInterval(pingInterval);
      clearInterval(fingerprintInterval);
      onClose();
    };


    const instance = new SignalingConnection(socket);

    socket.onmessage = (event) => {
      // oeca: an SDP is at most 128 KiB; nothing a peer sends needs this much
      if (typeof event.data !== "string" || event.data.length > MAX_SIGNALING_MESSAGE) return;
      const message = parseMessage(event.data);
      if (!message) return;
      if (
        message.type === "ANSWER" &&
        instance._onAnswer &&
        message.sessionId === instance._onAnswer.sessionId &&
        // oeca: any peer can send an ANSWER with any session id
        message.peer?.id === instance._onAnswer.from
      ) {
        instance._onAnswer.callback(message);
        instance._onAnswer = null;
      }
      onMessage(message);
    };

    return instance;
  }

  /** oeca: drop this connection (the loop reconnects), e.g. after the tab slept. */
  public close() {
    this._socket.close();
  }

  public send(message: WsClientMessage) {
    this._socket.send(JSON.stringify(message));
  }

  public async waitForAnswer(sessionId: string, from: string): Promise<AnswerMessage> {
    return await new Promise<AnswerMessage>((resolve) => {
      this._onAnswer = {
        sessionId,
        from,
        callback: (message) => resolve(message),
      };
    });
  }

  public async waitUntilClose(): Promise<void> {
    return new Promise((resolve) => {
      this._socket.addEventListener("close", () => {
        resolve();
      });
    });
  }
}

/** oeca: null for what is no message (the server also sends plain text). */
export function parseMessage(text: string): WsServerMessage | null {
  let m: any;
  try {
    m = JSON.parse(text);
  } catch {
    return null;
  }
  const isPeer = (p: any) => typeof p?.id === "string" && typeof p.alias === "string";
  switch (m?.type) {
    case "HELLO":
      return isPeer(m.client) && Array.isArray(m.peers) && m.peers.every(isPeer) ? m : null;
    case "JOIN":
    case "UPDATE":
      return isPeer(m.peer) ? m : null;
    case "LEFT":
      return typeof m.peerId === "string" ? m : null;
    case "OFFER":
    case "ANSWER":
      return isPeer(m.peer) && typeof m.sessionId === "string" && typeof m.sdp === "string" ? m : null;
    case "ERROR":
      return typeof m.code === "number" ? m : null;
    default:
      return null;
  }
}

type OnAnswer = {
  sessionId: string;
  from: string;
  callback: (message: AnswerMessage) => void;
};

export type ClientInfoWithoutId = {
  alias: string;
  version: string;
  deviceModel?: string;
  deviceType?: PeerDeviceType;
  token: string;
};

export type ClientInfo = ClientInfoWithoutId & { id: string };

export enum PeerDeviceType {
  mobile = "MOBILE",
  desktop = "DESKTOP",
  web = "WEB",
  headless = "HEADLESS",
  server = "SERVER",
}

export type WsServerMessage =
  | HelloMessage
  | JoinMessage
  | LeftMessage
  | UpdateMessage
  | OfferMessage
  | AnswerMessage
  | ErrorMessage;

export type HelloMessage = {
  type: "HELLO";
  client: ClientInfo;
  peers: ClientInfo[];
};

export type JoinMessage = {
  type: "JOIN";
  peer: ClientInfo;
};

export type UpdateMessage = {
  type: "UPDATE";
  peer: ClientInfo;
};

export type LeftMessage = {
  type: "LEFT";
  peerId: string;
};

export type WsServerSdpMessage = {
  peer: ClientInfo;
  sessionId: string;
  sdp: string;
};

export type OfferMessage = WsServerSdpMessage & { type: "OFFER" };

export type AnswerMessage = WsServerSdpMessage & { type: "ANSWER" };

export type ErrorMessage = {
  type: "ERROR";
  code: number;
};

type OnMessageCallback = (message: WsServerMessage) => void;

export type WsClientMessage = WsClientUpdateMessage | WsClientSdpMessage;

export type WsClientUpdateMessage = {
  type: "UPDATE";
  info: ClientInfoWithoutId;
};

export type WsClientSdpMessage = {
  type: "OFFER" | "ANSWER";
  sessionId: string;
  target: string;
  sdp: string;
};
