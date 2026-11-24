import { describe, expect, it } from "vitest";
import type { TransferView } from "~/services/transfer";
import type { FileState } from "~/services/store";
import {
  armsIn,
  confirmedCount,
  fileRows,
  identityBadge,
  noticeId,
  notchState,
  overallProgress,
  peerDetail,
  peerKind,
  peerName,
  privacyNotice,
  selectionIssues,
  stageTitle,
  statusLine,
} from "~/utils/view";

function transfer(over: Partial<TransferView> = {}): TransferView {
  return {
    id: "t1",
    direction: "receive",
    claimed: { peer: { id: "p1", alias: "Pixel" }, fingerprint: null },
    capability: "oeca",
    connection: { state: "claimed" },
    identity: null,
    stage: "approval",
    failure: null,
    decideAfter: null,
    offered: [],
    ...over,
  };
}

const file = (id: string, total: number, curr: number, state: FileState["state"], error?: string): FileState => ({
  id,
  name: `${id}.bin`,
  total,
  curr,
  state,
  error,
});

describe("status", () => {
  const base = { framed: false, crypto: true, started: true, link: "online" as const, online: true, limited: false, peers: 0 };
  it("says what the connection is, worst first", () => {
    expect(statusLine({ ...base, framed: true }).label).toBe("Unavailable");
    expect(statusLine({ ...base, crypto: false }).label).toBe("Not supported");
    expect(statusLine({ ...base, started: false }).label).toBe("Off");
    expect(statusLine(base)).toEqual({ label: "Searching", tone: "good", busy: true });
    expect(statusLine({ ...base, peers: 2 }).label).toBe("2 nearby");
    expect(statusLine({ ...base, online: false, link: "connecting" }).label).toBe("Connecting");
    expect(statusLine({ ...base, online: false, link: "reconnecting" }).label).toBe("Reconnecting");
    expect(statusLine({ ...base, online: false, link: "reconnecting", limited: true }).label).toBe("Server busy");
  });
  it("names kinds of device without trusting them", () => {
    expect(peerKind("mobile").label).toBe("Phone");
    expect(peerKind("web").icon).toBe("browser");
    expect(peerKind(undefined).label).toBe("Unknown device");
  });
});

describe("identity", () => {
  it("is distinct for every trust state and never comes from the name", () => {
    const cases: [Partial<TransferView>, string][] = [
      [{ identity: { state: "trusted", name: "My PC", nameMismatch: false } }, "Trusted"],
      [{ identity: { state: "new" } }, "New device"],
      [{ identity: { state: "new" }, capability: "stock" }, "Unverifiable"],
      [{ identity: { state: "changed" } }, "Identity changed"],
      [{ identity: { state: "revoked" } }, "Forgotten device"],
      [{ identity: null }, "Not verified yet"],
    ];
    for (const [over, label] of cases) {
      // A claimed alias equal to a trusted name changes nothing
      const t = transfer({ ...over, claimed: { peer: { id: "p", alias: "My PC" }, fingerprint: null } });
      expect(identityBadge(t)?.label).toBe(label);
    }
    expect(identityBadge(transfer({ connection: { state: "closed" } }))).toBeNull();
  });
  it("shows the confirmed name of a trusted device, and quotes a claim", () => {
    expect(peerName(transfer({ identity: { state: "trusted", name: "My PC", nameMismatch: true } }))).toBe("My PC");
    expect(peerName(transfer())).toBe("“Pixel”");
    expect(peerName(transfer({ claimed: { peer: { id: "p", alias: "" }, fingerprint: null } }))).toBe("“Unnamed device”");
  });
  it("keeps declines, cancellations and failures apart", () => {
    expect(stageTitle(transfer({ stage: "declined", direction: "send" }))).toBe("Declined by the other device");
    expect(stageTitle(transfer({ stage: "declined" }))).toBe("You declined");
    expect(stageTitle(transfer({ stage: "cancelled" }))).toBe("Cancelled");
    expect(stageTitle(transfer({ stage: "failed", direction: "send" }))).toBe("Couldn't send");
    expect(stageTitle(transfer({ stage: "done", direction: "send" }))).toBe("Sent");
  });
});

describe("arming", () => {
  it("waits until the question was shown and armed", () => {
    expect(armsIn(transfer({ decideAfter: null }), 1000)).toBeNull();
    expect(armsIn(transfer({ decideAfter: 1700 }), 1000)).toBe(700);
    expect(armsIn(transfer({ decideAfter: 1700 }), 2000)).toBe(0);
    // Nothing is asked while transferring: Cancel never waits
    expect(armsIn(transfer({ stage: "transferring", decideAfter: null }), 0)).toBe(0);
  });
});

describe("files", () => {
  it("never calls a file sent before the receiver confirmed it", () => {
    const files = {
      a: file("a", 100, 100, "finished"),
      b: file("b", 100, 100, "sending"), // every byte left, no confirmation yet
      c: file("c", 200, 50, "sending"),
      d: file("d", 50, 0, "pending"),
    };
    const rows = fileRows(files, "send", "transferring");
    expect(rows.map((r) => r.label)).toEqual(["Saved by the other device", "Waiting for confirmation", "25%", "Queued"]);
    expect(rows.map((r) => r.done)).toEqual([true, false, false, false]);
    // Sending: only confirmed bytes count (a's 100 of 450)
    expect(overallProgress(files, "send")).toBeCloseTo(100 / 450);
    // Receiving: what arrived here counts
    expect(overallProgress(files, "receive")).toBeCloseTo(250 / 450);
    expect(confirmedCount(files)).toEqual({ done: 1, all: 4 });
  });
  it("reports each file's outcome when a transfer ends part way", () => {
    const files = {
      a: file("a", 10, 10, "finished"),
      b: file("b", 10, 3, "error", "disk full"),
      c: file("c", 10, 0, "pending"),
      d: file("d", 10, 0, "skipped"),
    };
    const rows = fileRows(files, "receive", "failed");
    expect(rows.map((r) => [r.label, r.tone])).toEqual([
      ["Received", "good"],
      ["Not saved: disk full", "bad"],
      ["Not confirmed", "warn"],
      ["Not wanted by the other device", "plain"],
    ]);
    expect(overallProgress({ e: file("e", 0, 0, "finished"), f: file("f", 0, 0, "pending") }, "send")).toBe(0.5);
  });
  it("checks the files chosen to send", () => {
    expect(selectionIssues([]).blocking).toMatch(/at least one/);
    const ok = selectionIssues([{ name: "a.jpg", size: 3 }]);
    expect(ok).toEqual({ blocking: null, notes: [] });
    const risky = selectionIssues([
      { name: "setup.exe", size: 3 },
      { name: "empty.txt", size: 0 },
    ]);
    expect(risky.blocking).toBeNull();
    expect(risky.notes[0]).toMatch(/1 file can run programs/);
    expect(risky.notes[1]).toMatch(/1 empty file/);
    const many = Array.from({ length: 101 }, (_, i) => ({ name: `${i}.txt`, size: 1 }));
    expect(selectionIssues(many).blocking).toMatch(/up to 100/);
  });
});

describe("privacy notice", () => {
  const text = privacyNotice("Blue Fox")
    .map((p) => p.text)
    .join(" ");
  it("says what the code does, and promises nothing it can't", () => {
    for (const fact of [
      "public.localsend.org",
      "stun.localsend.org",
      "not by Oeca",
      "“Blue Fox”",
      "IPv4 address or IPv6 /64",
      "can include strangers",
      "doesn't promise that only devices on your Wi-Fi",
      "nothing is relayed",
      "encrypted WebRTC connection",
      "Nothing is received without your approval",
      "never opened automatically",
      "can log requests",
    ]) {
      expect(text).toContain(fact);
    }
    expect(text).not.toMatch(/nothing is stored online|cannot see|only your Wi-?Fi/i);
  });
  it("has a stable id that changes with the text", () => {
    expect(noticeId()).toBe(noticeId());
    expect(noticeId()).toMatch(/^[0-9a-f]+$/);
  });
});

describe("notch", () => {
  const on = { label: "2 nearby", tone: "good" as const, busy: false };
  it("shows the connection, worst news first", () => {
    expect(notchState(on, false, null, {}).label).toBe("Hop is off");
    expect(notchState({ label: "Not supported", tone: "bad", busy: false }, false, null, {}).mode).toBe("bad");
    expect(notchState(on, true, null, {})).toEqual({ mode: "nearby", label: "2 nearby", progress: null });
    expect(notchState({ label: "Searching", tone: "good", busy: true }, true, null, {}).mode).toBe("searching");
    expect(notchState({ label: "Reconnecting", tone: "warn", busy: true }, true, null, {}).mode).toBe("warn");
    expect(notchState({ label: "Connecting", tone: "plain", busy: true }, true, null, {}).mode).toBe("quiet");
  });
  it("follows a transfer: asks, confirmed progress, and how it ended", () => {
    const files = { a: file("a", 100, 100, "finished"), b: file("b", 100, 100, "sending") };
    expect(notchState(on, true, transfer({ stage: "approval" }), {})).toEqual({ mode: "asks", label: "Incoming files", progress: null });
    expect(notchState(on, true, transfer({ stage: "verification" }), {}).label).toBe("Check this device");
    const moving = notchState(on, true, transfer({ stage: "transferring", direction: "send" }), files);
    expect(moving.label).toBe("Sending 1 of 2");
    expect(moving.progress).toBeCloseTo(0.5, 5); // b is sent, not confirmed
    expect(notchState(on, true, transfer({ stage: "done" }), files).label).toBe("Received");
    expect(notchState(on, true, transfer({ stage: "failed" }), files)).toEqual({ mode: "ended", label: "Didn't finish", progress: null });
    expect(notchState(on, true, transfer({ stage: "declined" }), files).label).toBe("Declined");
  });
  it("describes a device in plain words, from what it says it is", () => {
    expect(peerDetail("desktop", "Windows")).toBe("Windows computer");
    expect(peerDetail("web", "Safari")).toBe("Safari browser");
    expect(peerDetail("mobile", null)).toBe("Phone");
    expect(peerDetail(undefined, "Fridge")).toBe("Fridge");
  });
});
