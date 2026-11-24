import { afterEach, describe, expect, it, vi } from "vitest";
import { Authenticated, connected, initiate, readSdp, respondToExchange, usable } from "./proof";
import { SignalingConnection } from "./signaling";
import { certificateDigest, commitOf, OSV, randomSecret } from "./trust";
import { claim, fakePc, FakeChannel, fingerprint, sdpOf } from "./testing";

afterEach(() => vi.unstubAllGlobals());

describe("identity types", () => {
  it("cannot be made outside the proof", () => {
    expect(() => new (Authenticated as any)(Symbol("proof"), null, "000 000")).toThrow(TypeError);
  });

  it("takes only the connection's own certificate", async () => {
    const der = new TextEncoder().encode("a synthetic certificate").buffer as ArrayBuffer;
    const own = `sha-256 ${await certificateDigest(der)}`;
    expect((await connected(fakePc(own, der), claim(own))).fingerprint).toBe(own);
    const copied = fingerprint(5);
    await expect(connected(fakePc(copied, der), claim(copied))).rejects.toMatchObject({ code: "authentication" });
    await expect(connected(fakePc(own, null), claim(own))).rejects.toMatchObject({ code: "authentication" });
    await expect(connected(fakePc(own), claim(copied))).rejects.toMatchObject({ code: "authentication" });
  });
});

describe("what SDP says", () => {
  it.each(["a=x-oeca-nearby-nonce:short", "a=X-OECA-NEARBY-NONCE:abc", "a=x-oeca-nearby-v2:hello"])(
    "never takes a malformed Oeca peer for a stock one: %s",
    (line) => {
      const said = readSdp(sdpOf(fingerprint(1), `${line}\r\n`), OSV.NONCE);
      expect(said.capability).toBe("oeca");
      expect(() => usable(said)).toThrow(expect.objectContaining({ code: "authentication" }));
    },
  );

  it("keeps a busy peer Oeca, and proves nothing by it", () => {
    const said = readSdp(`v=0\r\na=${OSV.BUSY}:1\r\n`, OSV.NONCE);
    expect(said).toMatchObject({ capability: "oeca", busy: true, value: null, fingerprint: null });
  });
});

describe("the exchange names the selected peer", () => {
  function offerer() {
    const channel = new FakeChannel();
    const fp = fingerprint(1);
    const pc = Object.assign(fakePc(fp), { createDataChannel: () => channel });
    const secret = randomSecret();
    const x = { myId: "a", peerId: "m", claimed: claim(fp) };
    return { channel, secret, done: initiate(pc, x, secret, randomSecret()) };
  }

  it("reveals nothing to a peer behind a relay", async () => {
    const { channel, done } = offerer();
    channel.push({ ready: 1, peer: "b" });
    await expect(done).rejects.toMatchObject({ code: "authentication" });
    expect(channel.sent).toEqual([]);
  });

  it("reveals nothing to a peer that does not say who it is", async () => {
    const { channel, done } = offerer();
    channel.push({ ready: 1 });
    await expect(done).rejects.toMatchObject({ code: "authentication" });
    expect(channel.sent).toEqual([]);
  });

  it("completes with the selected peer, only after its acknowledgement", async () => {
    const { channel, secret, done } = offerer();
    channel.onSend = () => queueMicrotask(() => channel.push({ ok: 1 }));
    channel.push({ ready: 1, peer: "m" });
    expect((await done).code).toMatch(/^\d{3} \d{3}$/);
    expect(channel.sent).toEqual([{ r: secret, peer: "a" }]);
  });

  it.each([
    ["another sender", (r: string) => ({ r, peer: "a" })],
    ["no sender", (r: string) => ({ r })],
    ["another secret", () => ({ r: randomSecret(), peer: "m" })],
  ] as const)("is not acknowledged for %s", async (_label, reveal) => {
    const channel = new FakeChannel();
    const secret = randomSecret();
    const fp = fingerprint(1);
    channel.onSend = (m) => {
      if (m.ready) queueMicrotask(() => channel.push(reveal(secret)));
    };
    const x = { myId: "b", peerId: "m", claimed: claim(fp) };
    const ch = Promise.resolve(channel as unknown as RTCDataChannel);
    const done = respondToExchange(fakePc(fp), ch, x, await commitOf(secret), randomSecret());
    await expect(done).rejects.toMatchObject({ code: "authentication" });
    expect(channel.sent).toEqual([{ ready: 1, peer: "b" }]);
  });
});

describe("answers", () => {
  it("applies only the selected peer's", async () => {
    let socket: any;
    vi.stubGlobal(
      "WebSocket",
      class {
        constructor() {
          socket = this;
          queueMicrotask(() => socket.onopen());
        }
        send() {}
        close() {}
      },
    );
    vi.stubGlobal("setInterval", () => 0);
    vi.spyOn(console, "log").mockImplementation(() => {});
    const connection = await SignalingConnection.connect({
      url: "ws://synthetic",
      info: { alias: "me", version: "2.3", token: "" },
      onMessage() {},
      generateNewInfo: async () => ({ alias: "me", version: "2.3", token: "" }),
      onClose() {},
    });
    let applied: any = null;
    void connection.waitForAnswer("session", "target").then((a) => (applied = a));
    const answer = (from: string) =>
      socket.onmessage({ data: JSON.stringify({ type: "ANSWER", sessionId: "session", sdp: "x", peer: { id: from, alias: "Phone" } }) });
    answer("someone-else");
    await new Promise((r) => setTimeout(r, 0));
    expect(applied).toBeNull();
    answer("target");
    await new Promise((r) => setTimeout(r, 0));
    expect(applied.peer.id).toBe("target");
    vi.restoreAllMocks();
  });
});
