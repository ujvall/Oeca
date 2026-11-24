import { describe, expect, it } from "vitest";
import { parseMessage } from "./signaling";

const peer = { id: "p", alias: "Phone", version: "2.3", token: "" };

describe("signaling frames", () => {
  it.each([
    "Limit reached",
    "",
    "null",
    "7",
    "[]",
    '{"type":"OFFER"}',
    '{"type":"HELLO","peers":[]}',
    '{"type":"JOIN","peer":null}',
    '{"type":"UNKNOWN"}',
    '{"type":"ERROR"}',
  ])("drops what is no message: %s", (frame) => {
    expect(parseMessage(frame)).toBeNull();
  });

  it.each([
    { type: "HELLO", client: peer, peers: [peer] },
    { type: "JOIN", peer },
    { type: "UPDATE", peer },
    { type: "LEFT", peerId: "p" },
    { type: "OFFER", peer, sessionId: "s", sdp: "x" },
    { type: "ANSWER", peer, sessionId: "s", sdp: "x" },
    { type: "ERROR", code: 429 },
  ])("keeps a message: $type", (message) => {
    expect(parseMessage(JSON.stringify(message))).toEqual(message);
  });
});
