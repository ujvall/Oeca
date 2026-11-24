import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Devices } from "./devices";
import type { Capability } from "./proof";
import { authenticated, claim, FakeBrowser, fingerprint as key, plainConnection } from "./testing";
import { ARMING_MS, Board, MIN_PROGRESS, type Direction, type Gate } from "./transfer";

let browser: FakeBrowser;
let devices: Devices;
let board: Board;
let now: number;
let aborted: string[];

beforeEach(() => {
  browser = new FakeBrowser();
  devices = browser.devices();
  now = 1000;
  aborted = [];
  board = new Board(devices, () => {}, () => now);
});

async function trust(seed: number, name: string, alias = name) {
  await devices.pin(await authenticated(seed), name, alias);
}

function connecting(direction: Direction, alias: string, capability: Capability, seed: number): string {
  const id = board.begin(direction, { id: `id of ${alias}`, alias }, () => aborted.push(alias));
  board.claim(id, claim(key(seed)), capability);
  return id;
}

async function oeca(direction: Direction, alias: string, seed: number): Promise<{ id: string; gate: Gate }> {
  const id = connecting(direction, alias, "oeca", seed);
  return { id, gate: await board.admit(id, await authenticated(seed)) };
}

/** On screen for long enough. */
function show(id: string) {
  board.displayed(id, board.view()!.stage);
  now += ARMING_MS;
}

const files = [{ id: "0", fileName: "a.txt", size: 1 }];

async function noDecisionWorks(id: string, code: string) {
  await expect(board.verify(id, "Name")).rejects.toMatchObject({ code });
  await expect(board.confirmName(id)).rejects.toMatchObject({ code });
  await expect(board.respond(id, true)).rejects.toMatchObject({ code });
  await expect(board.respond(id, false)).rejects.toMatchObject({ code });
  await expect(board.allowOnce(id)).rejects.toMatchObject({ code });
}

describe("decisions belong to one transfer", () => {
  it("ignores a click meant for a replaced transfer", async () => {
    const shown = await oeca("receive", "Alice", 1);
    show(shown.id);
    board.end(shown.id, "cancelled");
    const attacker = await oeca("receive", "Mallory", 2);
    expect(attacker.id).not.toBe(shown.id);
    await noDecisionWorks(shown.id, "not-current");
    expect(() => board.cancel(shown.id)).toThrow();
    expect(board.view()!.stage).toBe("verification");
    expect(await devices.list()).toEqual([]);
  });

  it("waits until the question was seen, and again for the next one", async () => {
    const { id } = await oeca("receive", "Alice", 1);
    now += ARMING_MS * 9;
    await expect(board.verify(id, "Alice")).rejects.toMatchObject({ code: "not-yet" });
    board.displayed(id, "verification");
    now += ARMING_MS - 1;
    await expect(board.verify(id, "Alice")).rejects.toMatchObject({ code: "not-yet" });
    now += 1;
    await board.verify(id, "Alice");
    void board.offer(id, files);
    board.displayed(id, "verification"); // the previous question's
    now += ARMING_MS * 9;
    await expect(board.respond(id, true)).rejects.toMatchObject({ code: "not-yet" });
  });

  it.each(["oeca", "stock"] as const)("decides nothing while connecting (%s)", async (capability) => {
    await trust(1, "Phone");
    const id = connecting("receive", "Phone", capability, 1);
    show(id);
    await noDecisionWorks(id, "wrong-stage");
    expect(board.offer(id, files)).toBeNull();
    expect(board.view()!.offered).toEqual([]);
  });

  it("accepts only with the list on screen", async () => {
    await trust(1, "Phone");
    const { id, gate } = await oeca("receive", "Phone", 1);
    expect(gate).toBeNull();
    show(id);
    await noDecisionWorks(id, "wrong-stage");
    const approval = board.offer(id, files)!;
    show(id);
    await expect(board.allowOnce(id)).rejects.toBeDefined();
    await board.respond(id, true);
    expect(await approval.answer).toBe(true);
    await expect(board.respond(id, true)).rejects.toMatchObject({ code: "wrong-stage" });
  });

  it("does not take verifying for accepting", async () => {
    const { id, gate } = await oeca("receive", "Alice", 1);
    show(id);
    await expect(board.respond(id, true)).rejects.toMatchObject({ code: "wrong-stage" });
    await board.verify(id, "Alice");
    expect(await gate!.answer).toBe(true);
    expect(board.view()!.stage).toBe("awaiting-offer");
  });

  it("ignores a decision after the transfer ended", async () => {
    const { id, gate } = await oeca("send", "Alice", 1);
    show(id);
    board.kill(id, "failed");
    expect(await gate!.answer).toBe(false);
    await noDecisionWorks(id, "not-current");
    expect(await devices.list()).toEqual([]);
  });

  it("does not let a decision waiting for the store reach a replacement", async () => {
    const first = await oeca("receive", "Alice", 1);
    show(first.id);
    const slow = board.verify(first.id, "Alice");
    const refused = expect(slow).rejects.toMatchObject({ code: "not-current" });
    board.cancel(first.id);
    const second = await oeca("receive", "Mallory", 2);
    await refused;
    expect(await devices.list()).toEqual([]);
    expect(board.view()!.id).toBe(second.id);
    expect(board.view()!.stage).toBe("verification");
  });

  it("does not let an accept waiting for the store reach a replacement", async () => {
    await trust(1, "Phone");
    const first = await oeca("receive", "Phone", 1);
    void board.offer(first.id, files);
    show(first.id);
    const slow = board.respond(first.id, true);
    const refused = expect(slow).rejects.toMatchObject({ code: "not-current" });
    board.cancel(first.id);
    const second = await oeca("receive", "Phone", 1);
    void board.offer(second.id, files);
    await refused;
    expect(board.view()!.stage).toBe("approval");
  });
});

describe("gates", () => {
  it("reports a refusal as a refusal, and admits without waiting for it", async () => {
    const { id, gate } = await oeca("send", "Alice", 1);
    show(id);
    await board.respond(id, false);
    expect(await gate!.answer).toBe(false);
  });
});

describe("names belong to keys", () => {
  it("holds a trusted key under another alias, with its confirmed name", async () => {
    await trust(1, "Bob's laptop", "Bob");
    const { id, gate } = await oeca("send", "Mallory", 1);
    expect(gate).not.toBeNull();
    expect(board.view()).toMatchObject({
      stage: "name-check",
      identity: { state: "trusted", name: "Bob's laptop", nameMismatch: true },
    });
    show(id);
    await board.confirmName(id);
    expect(await gate!.answer).toBe(true);
    expect((await devices.list())[0]?.name).toBe("Bob's laptop");
  });

  it("warns about a spoofed alias and replaces nobody", async () => {
    await trust(1, "Phone");
    const { id } = await oeca("receive", "Phone", 2);
    expect(board.view()!.identity).toEqual({ state: "changed" });
    show(id);
    await expect(board.verify(id, "Phone")).rejects.toMatchObject({ trust: { code: "name-in-use" } });
    await board.verify(id, "New phone");
    expect((await devices.list()).map((d) => d.state)).toEqual(["trusted", "trusted"]);
  });
});

describe("claimed is not authenticated", () => {
  it("gives a claimed fingerprint no identity", async () => {
    await trust(1, "Phone");
    connecting("send", "Phone", "oeca", 1);
    const t = board.view()!;
    expect(t.connection).toEqual({ state: "claimed" });
    expect(t.identity).toBeNull();
    expect(JSON.stringify(t)).not.toContain("trusted");
  });

  it("takes no look-alike object for a proof", async () => {
    await trust(1, "Phone");
    const id = connecting("send", "Phone", "oeca", 1);
    await expect(board.admit(id, { fingerprint: key(1), code: "000 000" } as any)).rejects.toBeDefined();
    expect(board.view()!.identity).toBeNull();
  });

  it("fails an Oeca peer without the proof", async () => {
    await trust(1, "Phone");
    const id = connecting("send", "Phone", "oeca", 1);
    await expect(board.admit(id, await plainConnection(1))).rejects.toMatchObject({ code: "authentication" });
    expect(board.view()).toMatchObject({ capability: "oeca", identity: null, connection: { state: "closed" } });
  });

  it.each(["failed", "cancelled", "declined"] as const)("leaves no trust behind when %s", async (how) => {
    await trust(1, "Phone");
    const { id } = await oeca("send", "Phone", 1);
    expect(board.view()!.connection.state).toBe("authenticated");
    board.end(id, how);
    expect(board.view()).toMatchObject({ stage: how, identity: null, connection: { state: "closed" } });
    board.end(id, "done");
    expect(board.view()!.stage).toBe(how);
  });
});

describe("revocation", () => {
  it("ends a live transfer at once and shuts its gate", async () => {
    await trust(1, "Phone");
    const { id, gate } = await oeca("send", "Other name", 1);
    show(id);
    const revoking = devices.forget(key(1));
    // Before the write has even started
    expect(board.view()).toMatchObject({ stage: "failed", failure: { code: "revoked" }, identity: null });
    expect(aborted).toEqual(["Other name"]);
    expect(await gate!.answer).toBe(false);
    await revoking;
  });

  it("ends it even if the revocation cannot be saved", async () => {
    await trust(1, "Phone");
    const { id } = await oeca("send", "Phone", 1);
    browser.faults.write = true;
    await expect(devices.forget(key(1))).rejects.toMatchObject({ code: "not-saved" });
    expect(board.isLive(id)).toBe(false);
    expect((await devices.standing(key(1), "Phone")).kind).toBe("revoked");
  });

  it("ends it when another tab revoked", async () => {
    await trust(1, "Phone");
    const { id } = await oeca("send", "Phone", 1);
    await browser.devices().forget(key(1));
    await expect(board.assertAllowed(id)).rejects.toMatchObject({ code: "revoked" });
    await board.recheck();
    expect(board.isLive(id)).toBe(false);
  });

  it("refuses a pending approval after a revocation elsewhere", async () => {
    await trust(1, "Phone");
    const { id } = await oeca("receive", "Phone", 1);
    void board.offer(id, files);
    show(id);
    await browser.devices().forget(key(1));
    await expect(board.respond(id, true)).rejects.toMatchObject({ code: "not-eligible" });
  });

  it("opens nothing for a pin that was not saved", async () => {
    const { id } = await oeca("send", "Alice", 1);
    show(id);
    browser.faults.write = true;
    await expect(board.verify(id, "Alice")).rejects.toMatchObject({ trust: { code: "write-failed" } });
    expect(board.view()).toMatchObject({ stage: "verification", identity: { state: "new" } });
  });

  it("shows a returning revoked device as revoked", async () => {
    await trust(1, "Phone");
    await devices.forget(key(1));
    const { id } = await oeca("receive", "Brand new name", 1);
    expect(board.view()).toMatchObject({ stage: "verification", identity: { state: "revoked" } });
    show(id);
    await expect(board.allowOnce(id)).rejects.toMatchObject({ code: "wrong-stage" });
    await board.verify(id, "Phone again"); // explicit, with the code
    expect(board.view()!.identity?.state).toBe("trusted");
  });
});

describe("capability is not identity", () => {
  it.each([
    [1, "Phone", null],
    [2, "Tablet", "revoked"],
    [3, "Phone", "changed"],
    [4, "Tablet", "changed"],
  ] as const)("keeps the warning without the extension (%i, %s)", async (seed, alias, left) => {
    await trust(1, "Phone");
    await trust(2, "Tablet");
    await devices.forget(key(2));
    for (const direction of ["send", "receive"] as const) {
      const id = connecting(direction, alias, "stock", seed);
      await expect(board.admit(id, await plainConnection(seed))).rejects.toMatchObject({ code: "downgraded" });
      expect(board.view()!.identity?.state ?? null).toBe(left);
      await noDecisionWorks(id, "not-current");
    }
  });

  it.each(["send", "receive"] as const)("allows an unknown stock peer once (%s)", async (direction) => {
    const id = connecting(direction, "Someone", "stock", 7);
    let gate = await board.admit(id, await plainConnection(7));
    if (direction === "receive") gate = board.offer(id, files);
    show(id);
    await expect(board.verify(id, "Someone")).rejects.toBeDefined();
    await expect(board.respond(id, true)).rejects.toBeDefined();
    await board.allowOnce(id);
    expect(await gate!.answer).toBe(true);
    expect(board.view()!.connection.state).toBe("connected");
    expect(await devices.list()).toEqual([]);
  });
});

describe("deadlines", () => {
  afterEach(() => vi.useRealTimers());

  it.each(["signaling", "connecting"] as const)("ends a transfer stuck in %s", (stage) => {
    vi.useFakeTimers();
    const id = board.begin("send", { id: "p", alias: "P" }, () => aborted.push("P"));
    if (stage === "connecting") board.claim(id, claim(key(1)), "stock");
    vi.advanceTimersByTime(76_000);
    expect(board.view()!.stage).toBe("failed");
    expect(aborted).toEqual(["P"]);
  });

  it("does not take a drip for progress", async () => {
    await trust(1, "Phone");
    const { id } = await oeca("send", "Phone", 1);
    vi.useFakeTimers();
    board.sending(id);
    board.progress(id, 1);
    for (let i = 0; i < 3; i++) {
      vi.advanceTimersByTime(50_000);
      board.progress(id, 2 + i);
    }
    expect(board.view()!.stage).toBe("failed");
  });

  it("lets a moving transfer run", async () => {
    await trust(1, "Phone");
    const { id } = await oeca("send", "Phone", 1);
    vi.useFakeTimers();
    board.sending(id);
    for (let i = 1; i <= 20; i++) {
      vi.advanceTimersByTime(100_000);
      board.progress(id, i * MIN_PROGRESS);
    }
    expect(board.view()!.stage).toBe("transferring");
  });

  it("lets the user's send replace a stock peer that asked nothing", async () => {
    const id = connecting("receive", "Someone", "stock", 7);
    await board.admit(id, await plainConnection(7));
    expect(board.view()!.stage).toBe("awaiting-offer");
    expect(board.unprovenIncoming()).toBe(id);
  });

  it("keeps the place of one that is asking", async () => {
    await oeca("receive", "Someone", 1);
    expect(board.unprovenIncoming()).toBeNull();
  });

  it("closes what ended but never finished closing", () => {
    vi.useFakeTimers();
    const id = board.begin("receive", { id: "p", alias: "P" }, () => aborted.push("P"));
    board.end(id, "declined");
    expect(aborted).toEqual([]);
    vi.advanceTimersByTime(6000);
    expect(aborted).toEqual(["P"]);
  });
});
