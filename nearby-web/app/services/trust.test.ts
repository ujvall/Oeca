import { describe, expect, it, vi } from "vitest";
import { authenticated, FakeBrowser } from "./testing";
import {
  canonicalFingerprint,
  commitOf,
  deleteIdentity,
  isSecret,
  loadCertificate,
  stableAlias,
  fingerprintFromSdp,
  randomSecret,
  sdpAttr,
  verificationCode,
  withSessionLines,
} from "./trust";

// Shared with Oeca's src/nearby.rs (verification_vector_matches_the_browser)
const FP_A =
  "sha-256 e0:79:e1:b9:10:34:7e:e7:3b:75:0c:ab:75:bc:b7:b0:0f:27:fe:fc:91:79:a8:c5:7d:14:1b:df:78:ae:26:e6";
const FP_B =
  "sha-256 58:41:72:c7:fe:91:8f:d4:b1:90:80:2e:16:13:7f:da:99:56:f8:5e:58:b4:d6:f3:7d:fa:ea:30:6c:5e:a4:9b";
const R_O = "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA";
const R_A = "ISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0-P0A";

describe("Oeca Nearby Verification v1", () => {
  it("matches the native implementation", async () => {
    expect(await commitOf(R_O)).toBe("VPyCJA6QvLU-7r7ZRl4u0bm4tVpOoHOx9FYAtdCFaaU");
    expect(await verificationCode(FP_A, FP_B, R_O, R_A)).toBe("346 672");
    expect(await verificationCode(FP_B, FP_A, R_O, R_A)).toBe("346 672");
    expect(await verificationCode(FP_A, FP_B, R_A, R_O)).not.toBe("346 672");
    expect(randomSecret()).toHaveLength(43);
  });

  it("reads and writes the SDP lines strictly", () => {
    const sdp = "v=0\r\ns=-\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:0\r\n";
    const out = withSessionLines(sdp, [`a=x-oeca-nearby-commit:${R_O}`]);
    expect(out.indexOf("a=x-oeca-nearby-commit")).toBeLessThan(out.indexOf("m=application"));
    expect(sdpAttr(out, "x-oeca-nearby-commit")).toBe(R_O);
    expect(sdpAttr("a=x-oeca-nearby-commit:abc", "x-oeca-nearby-commit")).toBeNull();
    expect(sdpAttr("v=0", "x-oeca-nearby-nonce")).toBeNull();
  });

  it("refuses ambiguous fingerprints", () => {
    for (const sdp of [
      `v=0\r\na=fingerprint:${FP_A}\r\nm=application 9\r\na=fingerprint:${FP_B}\r\n`,
      `m=application 9\r\na=fingerprint:${FP_A}\r\na=fingerprint:${FP_B}\r\n`,
      `v=0\r\n a=fingerprint:${FP_A}\r\n`,
      `a=fingerprint:${FP_A}\r\na=Fingerprint:${FP_B}\r\n`,
      `a=fingerprint:${FP_A}\ra=fingerprint:${FP_B}\r\n`,
    ]) {
      expect(fingerprintFromSdp(sdp)).toBeNull();
    }
    expect(fingerprintFromSdp(`a=fingerprint:${FP_A}\r\nm=x\r\na=fingerprint:${FP_A}\r\n`)).toBe(FP_A);
  });

  it("reads and normalizes the SDP fingerprint", () => {
    const sdp = `v=0\r\na=fingerprint:SHA-256 ${FP_A.slice(8).toUpperCase()}\r\n`;
    expect(fingerprintFromSdp(sdp)).toBe(FP_A);
    expect(fingerprintFromSdp("a=fingerprint:sha-1 ab:cd")).toBeNull();
    expect(fingerprintFromSdp(`a=fingerprint:sha-256 ${"z".repeat(95)}`)).toBeNull();
  });
});

// oeca: resetting this browser's identity (store.ts resetIdentity) deletes only the identity
describe("identity reset", () => {
  it("deletes this browser's certificate and alias, and nothing else", async () => {
    const browser = new FakeBrowser();
    let made = 0;
    vi.stubGlobal("indexedDB", browser.indexedDB);
    vi.stubGlobal("localStorage", browser.storage);
    vi.stubGlobal("RTCPeerConnection", {
      generateCertificate: async () => ({ id: ++made, expires: Date.now() + 1e10 }),
    });
    try {
      const first = await loadCertificate();
      expect(first.state).toBe("new");
      expect(stableAlias(() => "Alias one")).toBe("Alias one");
      await browser.devices().pin(await authenticated(1), "Phone", "Phone");
      browser.storage.setItem("unrelated", "kept");
      expect((await loadCertificate()).state).toBe("kept");

      await deleteIdentity();

      const after = await loadCertificate();
      expect(after.state).toBe("new");
      expect((after.cert as any).id).not.toBe((first.cert as any).id);
      expect(stableAlias(() => "Alias two")).toBe("Alias two");
      expect(browser.storage.getItem("unrelated")).toBe("kept");
      // The trusted devices are another record: they did not change
      expect(await browser.devices().list()).toHaveLength(1);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

// oeca: identity creation across tabs, and storage that fails (Step D3)
describe("identity across tabs and failures", () => {
  function setup() {
    const browser = new FakeBrowser();
    let made = 0;
    vi.stubGlobal("indexedDB", browser.indexedDB);
    vi.stubGlobal("localStorage", browser.storage);
    vi.stubGlobal("RTCPeerConnection", {
      generateCertificate: async () => ({ id: ++made, expires: Date.now() + 1e10 }),
    });
    return browser;
  }

  it("two tabs opened together end with one identity", async () => {
    const browser = setup();
    try {
      const [a, b] = await Promise.all([loadCertificate(), loadCertificate()]);
      expect((a.cert as any).id).toBe((b.cert as any).id);
      expect([a.state, b.state].sort()).toEqual(["kept", "new"]);
      expect((browser.data.get("dtls") as any).id).toBe((a.cert as any).id);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("a write that does not commit is never taken as kept, and leaves nothing half done", async () => {
    const browser = setup();
    try {
      browser.faults.write = true; // the transaction aborts (quota, a closing tab)
      const failed = await loadCertificate();
      expect(failed.state).toBe("unsaved");
      expect(browser.data.has("dtls")).toBe(false);
      browser.faults.write = false;
      const next = await loadCertificate(); // the reload after it
      expect(next.state).toBe("new");
      expect((next.cert as any).id).not.toBe((failed.cert as any).id);
      browser.faults.open = true; // storage gone (private mode, blocked)
      expect((await loadCertificate()).state).toBe("unsaved");
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("extension lines", () => {
  const n = "x-oeca-nearby-commit";
  const short = R_O.slice(0, 42);
  const plus = `+${R_O.slice(1)}`;
  const trail = `${R_O.slice(0, 42)}B`;
  // oeca: the same vectors as src/nearby.rs extension_lines_share_one_grammar_with_the_browser
  it.each([
    [`v=0\r\na=${n}:${R_O}\r\nm=application 9\r\n`, true, "CRLF, before m="],
    [`v=0\na=${n}:${R_O}\nm=application 9\n`, true, "LF only"],
    [`v=0\r\na=${n}:${R_O}\r\n`, true, "no media section"],
    [`v=0\r\na=${n}:${R_O} \r\nm=x\r\n`, false, "trailing space"],
    [`v=0\r\n a=${n}:${R_O}\r\n`, false, "leading space"],
    [`v=0\r\n\uFEFFa=${n}:${R_O}\r\n`, false, "BOM"],
    [`v=0\r\na=${n}:${R_O}\u0085\r\n`, false, "NEL"],
    [`v=0\r\na=${n}:${R_O}\r\na=${n}:${R_O}\r\nm=x\r\n`, false, "twice"],
    [`v=0\r\nm=x\r\na=${n}:${R_O}\r\n`, false, "media level only"],
    [`v=0\r\na=${n}:${R_O}\r\nm=x\r\na=${n}:${R_O}\r\n`, false, "session and media"],
    [`v=0\r\na=${n}:${R_O}=\r\n`, false, "padded"],
    [`v=0\r\na=${n}:${plus}\r\n`, false, "standard alphabet"],
    [`v=0\r\na=${n}:${trail}\r\n`, false, "non-canonical last character"],
    [`v=0\r\na=${n}:${R_O}\rx\r\n`, false, "lone CR"],
    [`v=0\r\na=${n}:${short}\r\n`, false, "42 characters"],
    [`v=0\r\na=${n}:${R_O}A\r\n`, false, "44 characters"],
    [`v=0\r\nA=${n}:${R_O}\r\n`, false, "upper-case a="],
    [`v=0\r\na=${n}:${R_O}\r\r\n`, false, "two CRs"],
  ])("extension lines: %j is %s (%s)", (sdp, ok) => {
    expect(sdpAttr(sdp, n)).toBe(ok ? R_O : null);
  });

  const fp = FP_A;
  const hex = FP_A.slice(8);
  const upper = FP_A.toUpperCase();
  const shortHex = FP_A.slice(8, -3);
  // the same vectors as src/nearby.rs fingerprints_share_one_grammar_with_the_browser
  it.each([
    [`${fp}`, true, "canonical"],
    [`${upper}`, true, "upper case"],
    [`sha-256\t${hex}`, true, "tab"],
    [`sha-256  ${hex}`, true, "two spaces"],
    [` ${fp} `, true, "spaces around"],
    [`sha-256\u0085${hex}`, false, "NEL"],
    [`sha-256\u00A0${hex}`, false, "no-break space"],
    [`\uFEFF${fp}`, false, "BOM"],
    [`sha-1 ${hex}`, false, "another digest"],
    [`sha-256 ${shortHex}`, false, "31 bytes"],
  ])("fingerprints: %j is %s (%s)", (text, ok) => {
    expect(canonicalFingerprint(text)).toBe(ok ? FP_A : null);
  });

  it("a secret is canonical unpadded URL-safe base64 of 32 bytes", () => {
    expect(isSecret(R_O)).toBe(true);
    for (const v of [short, `${R_O}A`, plus, trail, `${R_O}=`, ` ${R_O}`, `${R_O}
`]) expect(isSecret(v), v).toBe(false);
  });
});
