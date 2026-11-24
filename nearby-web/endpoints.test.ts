import { describe, expect, it } from "vitest";
import { endpoints, LOOPBACK_SIGNALING, PUBLIC_SIGNALING, PUBLIC_STUN } from "./endpoints";

describe("endpoints", () => {
  it("only a production build uses the public servers by default", () => {
    expect(endpoints({ NODE_ENV: "production" })).toEqual({
      signalingUrl: PUBLIC_SIGNALING,
      stunServers: [PUBLIC_STUN],
    });
    for (const env of [{}, { NODE_ENV: "development" }, { NODE_ENV: "test" }]) {
      const e = endpoints(env);
      expect(e.signalingUrl).toBe(LOOPBACK_SIGNALING);
      expect(e.stunServers).toEqual([]);
      expect(JSON.stringify(e)).not.toMatch(/localsend\.org/);
    }
  });
  it("follows explicit settings", () => {
    expect(endpoints({ NODE_ENV: "production", SIGNALING_URL: "ws://127.0.0.1:4000/", STUN_SERVERS: "" })).toEqual({
      signalingUrl: "ws://127.0.0.1:4000/",
      stunServers: [],
    });
    expect(endpoints({ SIGNALING_URL: PUBLIC_SIGNALING, STUN_SERVERS: "stun:a,stun:b" }).stunServers).toEqual([
      "stun:a",
      "stun:b",
    ]);
  });
});
