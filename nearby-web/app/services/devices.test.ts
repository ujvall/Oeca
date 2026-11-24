import { beforeEach, describe, expect, it } from "vitest";
import { cleanName, MAX_ATTEMPTS, MAX_ATTEMPTS_PER_PEER, MAX_RECORDS, MAX_TRUSTED, ATTEMPT_WINDOW_MS } from "./devices";
import { authenticated, FakeBrowser, fingerprint as key } from "./testing";

let browser: FakeBrowser;
beforeEach(() => {
  browser = new FakeBrowser();
});

async function pin(seed: number, name: string, alias = name) {
  await browser.devices().pin(await authenticated(seed), name, alias);
}

function seedStore(trusted: number, revoked: number) {
  const devices = Array.from({ length: trusted + revoked }, (_, i) => ({
    key: key(i + 1),
    name: `device ${i}`,
    alias: `alias ${i}`,
    verified: 1,
    revoked: i < revoked ? 2 : null,
  }));
  browser.data.set("records", { version: 1, devices, attempts: [] });
}

describe("names", () => {
  it("keeps the confirmed name apart from the peer's own", async () => {
    await pin(1, "Bob's laptop", "DESKTOP-1234");
    const d = browser.devices();
    expect(await d.standing(key(1), "bob's  LAPTOP")).toEqual({ kind: "trusted", name: "Bob's laptop", conflict: false });
    expect(await d.standing(key(1), "Mum's phone")).toEqual({ kind: "trusted", name: "Bob's laptop", conflict: true });
    expect((await d.list())[0]?.name).toBe("Bob's laptop");
  });

  it("never replaces a record because of a shared name", async () => {
    await pin(1, "Tablet");
    const d = browser.devices();
    expect((await d.standing(key(2), "tab​let‮")).kind).toBe("changed");
    await expect(d.pin(await authenticated(2), " tablet ", "Tablet")).rejects.toMatchObject({ code: "name-in-use" });
    await pin(2, "Other tablet", "Tablet");
    expect(await d.standing(key(1), "Tablet")).toEqual({ kind: "trusted", name: "Tablet", conflict: true });
    expect(await d.list()).toHaveLength(2);
    await expect(d.pin(await authenticated(3), " \n‮", "x")).rejects.toMatchObject({ code: "invalid-name" });
    expect(cleanName("A\tB\u0000\nC﻿")).toBe("A B C");
  });

  it("pins nothing that is not an authenticated identity", async () => {
    const forged = { fingerprint: key(1), code: "123 456" } as any;
    await expect(browser.devices().pin(forged, "Phone", "Phone")).rejects.toMatchObject({ code: "not-eligible" });
  });
});

describe("revocation", () => {
  it("survives a reload and keeps the name known", async () => {
    await pin(1, "Phone", "Pixel");
    await browser.devices().forget(key(1));
    const d = browser.devices();
    expect((await d.standing(key(1), "Any new name")).kind).toBe("revoked");
    expect((await d.standing(key(2), "Pixel")).kind).toBe("changed");
    expect((await d.standing(key(2), "Phone")).kind).toBe("changed");
    await expect(d.forget(key(9))).rejects.toMatchObject({ code: "not-found" });
    expect(await d.status()).toBe("ready");
  });

  it("is effective when the transaction fails, and across a reload", async () => {
    await pin(1, "Phone");
    await pin(2, "Tablet");
    const d = browser.devices();
    browser.faults.write = true;
    await expect(d.forget(key(1))).rejects.toMatchObject({ code: "not-saved", held: true });
    for (const tab of [d, browser.devices()]) {
      expect((await tab.standing(key(1), "Phone")).kind).toBe("revoked");
      expect((await tab.standing(key(2), "Tablet")).kind).toBe("unavailable");
      expect(await tab.status()).toBe("paused");
      expect((await tab.list()).find((x) => x.fingerprint === key(1))?.state).toBe("revocation-pending");
    }
    browser.faults = {};
    await browser.devices().retry();
    const after = browser.devices();
    expect((await after.standing(key(1), "Phone")).kind).toBe("revoked");
    expect((await after.standing(key(2), "Tablet")).kind).toBe("trusted");
    expect(await after.status()).toBe("ready");
  });

  it("holds everything when only an empty hold fits", async () => {
    await pin(1, "Phone");
    browser.faults = { write: true, storage: "full" };
    await expect(browser.devices().forget(key(1))).rejects.toMatchObject({ code: "not-saved", held: true });
    browser.faults = {};
    const reloaded = browser.devices();
    expect((await reloaded.standing(key(1), "Phone")).kind).toBe("unavailable");
    await expect(reloaded.retry()).rejects.toMatchObject({ code: "paused" });
    await expect(reloaded.pin(await authenticated(2), "New", "New")).rejects.toMatchObject({ code: "paused" });
    await reloaded.forgetAll();
    expect(await reloaded.status()).toBe("ready");
    expect((await browser.devices().standing(key(1), "Phone")).kind).toBe("revoked");
  });

  it("says so when nothing at all could be written", async () => {
    await pin(1, "Phone");
    const d = browser.devices();
    browser.faults = { write: true, storage: "dead" };
    await expect(d.forget(key(1))).rejects.toMatchObject({ code: "not-saved", held: false });
    expect((await d.standing(key(1), "Phone")).kind).toBe("revoked");
    browser.faults = {};
    await d.retry();
    expect((await browser.devices().standing(key(1), "Phone")).kind).toBe("revoked");
  });

  it("saves a revocation under a damaged hold and stays paused", async () => {
    await pin(1, "Phone");
    browser.kept.set("oeca-nearby-trust-hold", "garbage");
    await browser.devices().forget(key(1));
    const d = browser.devices();
    expect((await d.standing(key(1), "Phone")).kind).toBe("revoked");
    expect(await d.status()).toBe("paused");
    expect(browser.kept.get("oeca-nearby-trust-hold")).toBe("garbage");
  });

  it("reaches another tab before it is saved", async () => {
    await pin(1, "Phone");
    const [here, there] = [browser.devices(), browser.devices()];
    here.listen();
    there.listen();
    const heard = new Promise((resolve) => there.onChange((c) => c.revoked && resolve(c.revoked)));
    browser.faults = { write: true, storage: "dead" };
    await expect(here.forget(key(1))).rejects.toMatchObject({ code: "not-saved" });
    expect(await heard).toEqual({ all: false, keys: [key(1)] });
    await expect.poll(async () => (await there.standing(key(1), "Phone")).kind).toBe("revoked");
    here.close();
    there.close();
  });
});

describe("storage", () => {
  it("does not trust a pin that was not committed", async () => {
    browser.faults.write = true;
    const d = browser.devices();
    await expect(d.pin(await authenticated(1), "Phone", "Phone")).rejects.toMatchObject({
      code: "write-failed",
      reason: "QuotaExceededError",
    });
    expect((await d.standing(key(1), "Phone")).kind).toBe("new");
  });

  it("fails closed and leaves a damaged store as it is", async () => {
    browser.data.set("records", { version: 1, devices: [{ key: "x" }], attempts: [] });
    const d = browser.devices();
    expect(await d.status()).toBe("unreadable");
    expect((await d.standing(key(1), "Phone")).kind).toBe("unavailable");
    await expect(d.pin(await authenticated(1), "Phone", "Phone")).rejects.toMatchObject({ code: "unreadable" });
    expect(await d.beginAttempt("p", "t")).toBe(false);
    await expect(d.forget(key(1))).rejects.toMatchObject({ code: "not-saved", held: true });
    expect(browser.data.get("records")).toEqual({ version: 1, devices: [{ key: "x" }], attempts: [] });
  });

  it("trusts nobody without IndexedDB", async () => {
    browser.faults.open = true;
    const d = browser.devices();
    expect((await d.standing(key(1), "x")).kind).toBe("unavailable");
    expect(await d.status()).toBe("unavailable");
    await expect(d.list()).rejects.toMatchObject({ code: "unavailable" });
  });

  it("loses no update of two tabs at once", async () => {
    const tabs = [browser.devices(), browser.devices()];
    await Promise.all(
      Array.from({ length: 16 }, async (_, i) => tabs[i % 2]!.pin(await authenticated(i + 1), `device ${i}`, "a")),
    );
    expect(await browser.devices().list()).toHaveLength(16);
    await Promise.all([tabs[0]!.forget(key(1)), tabs[1]!.forget(key(2))]);
    const revoked = (await browser.devices().list()).filter((d) => d.state === "revoked");
    expect(revoked.map((d) => d.fingerprint).sort()).toEqual([key(1), key(2)].sort());
  });
});

describe("the device list", () => {
  it("lists, forgets and forgets all without a connection", async () => {
    await pin(1, "Zed");
    await pin(2, "Amy");
    await pin(3, "Mum");
    const d = browser.devices();
    await d.forget(key(3));
    const list = await d.list();
    expect(list.map((x) => [x.name, x.state])).toEqual([
      ["Amy", "trusted"],
      ["Zed", "trusted"],
      ["Mum", "revoked"],
    ]);
    expect(list[0]?.short).toMatch(/^([0-9A-F]{4} ){3}[0-9A-F]{4}$/);
    expect(list[0]?.verifiedAt).toBe(browser.clock);
    await d.forgetAll();
    expect((await browser.devices().list()).map((x) => x.state)).toEqual(["revoked", "revoked", "revoked"]);
  });
});

describe("capacity", () => {
  it("takes the 256th and refuses the 257th without evicting", async () => {
    seedStore(MAX_TRUSTED - 1, 1);
    await pin(9001, "256th");
    await expect(pin(9002, "257th")).rejects.toMatchObject({ code: "full" });
    const d = browser.devices();
    expect((await d.standing(key(1), "x")).kind).toBe("revoked");
    expect(await d.list()).toHaveLength(MAX_TRUSTED + 1);
    await d.forget(key(9001));
    await pin(9002, "257th");
    expect((await d.standing(key(9001), "x")).kind).toBe("revoked");
  });

  it.each([256, 257, MAX_RECORDS - 1])("never evicts one of %i revoked records", async (revoked) => {
    seedStore(1, revoked);
    if (revoked + 1 === MAX_RECORDS) await expect(pin(99999, "one more")).rejects.toMatchObject({ code: "full" });
    else await pin(99999, "one more");
    await browser.devices().forget(key(revoked + 1)); // always possible
    const list = await browser.devices().list();
    expect(list.filter((d) => d.state === "revoked")).toHaveLength(revoked + 1);
  });
});

describe("attempts", () => {
  it("are limited per peer and in all, across tabs and reloads", async () => {
    const tabs = await Promise.all(
      Array.from({ length: MAX_ATTEMPTS_PER_PEER + 3 }, (_, i) => browser.devices().beginAttempt("greedy", `${i}`)),
    );
    expect(tabs.filter(Boolean)).toHaveLength(MAX_ATTEMPTS_PER_PEER);
    for (let i = 0; i < MAX_ATTEMPTS - MAX_ATTEMPTS_PER_PEER; i++) {
      expect(await browser.devices().beginAttempt(`other ${i}`, `o${i}`)).toBe(true);
    }
    expect(await browser.devices().beginAttempt("yet another", "x")).toBe(false);
    browser.clock -= 5000;
    expect(await browser.devices().beginAttempt("clock", "c")).toBe(false);
    browser.clock += 5000 + ATTEMPT_WINDOW_MS;
    expect(await browser.devices().beginAttempt("later", "l")).toBe(true);
  });

  it("are given back once decided, and refused if uncountable", async () => {
    const d = browser.devices();
    for (let i = 0; i < MAX_ATTEMPTS_PER_PEER * 3; i++) {
      expect(await d.beginAttempt("friend", `${i}`)).toBe(true);
      await d.refundAttempt(`${i}`);
    }
    browser.faults.write = true;
    expect(await d.beginAttempt("friend", "unsaved")).toBe(false);
  });
});
