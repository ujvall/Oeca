import pako from "pako";
import { dangerous, MAX_SAVED_NAME, nameParts, safeName } from "./limits";
import { describe, expect, it } from "vitest";
import { StreamController } from "../utils/streamController";
import {
  Approved,
  inflateSdp,
  MAX_REFUSALS,
  MAX_REFUSALS_PER_PEER,
  MAX_SDP_BYTES,
  parseFileList,
  RATE_LIMITED_WAIT_MS,
  reconnectIn,
  REFUSAL_WINDOW_MS,
  Refusals,
} from "./limits";

const deflate = (text: string) => pako.deflate(new TextEncoder().encode(text));
const file = (over: Record<string, unknown> = {}) => ({ id: "0", fileName: "a.txt", size: 3, fileType: "", ...over });

describe("inflating an SDP", () => {
  it("gives back what was deflated, up to the limit", () => {
    for (const text of ["", "v=0\r\n", "a".repeat(40_000), "x".repeat(MAX_SDP_BYTES)]) {
      expect(inflateSdp(deflate(text))).toBe(text);
    }
  });

  it("stops one byte past the limit", () => {
    expect(() => inflateSdp(deflate("x".repeat(MAX_SDP_BYTES + 1)))).toThrow("too large");
  });

  it("does not inflate a bomb", () => {
    const bomb = pako.deflate(new Uint8Array(16 * 1024 * 1024));
    expect(() => inflateSdp(bomb)).toThrow("too large");
  });

  it.each([
    ["truncated", () => deflate("v=0\r\n".repeat(500)).slice(0, 12)],
    ["garbage", () => new Uint8Array([1, 2, 3, 4])],
    ["empty", () => new Uint8Array()],
    ["not UTF-8", () => pako.deflate(new Uint8Array([0xff, 0xfe, 0xfd]))],
  ] as const)("refuses %s", (_label, bytes) => {
    expect(() => inflateSdp(bytes())).toThrow();
  });
});

describe("the file list", () => {
  it("keeps only its own fields", () => {
    const [parsed] = parseFileList([file({ metadata: { modified: "x" }, preview: "y" })]);
    expect(parsed).toEqual(file());
    expect(parseFileList([file({ size: 0, fileType: undefined })])[0]?.fileType).toBe("");
  });

  it.each([
    ["not a list", {}],
    ["empty", []],
    ["not an object", [null]],
    ["too many", Array.from({ length: 1001 }, (_, i) => file({ id: `${i}` }))],
    ["no id", [file({ id: "" })]],
    ["long id", [file({ id: "i".repeat(129) })]],
    ["numeric id", [file({ id: 0 })]],
    ["duplicate id", [file(), file({ size: 4096 })]],
    ["no name", [file({ fileName: undefined })]],
    ["long name", [file({ fileName: "n".repeat(1025) })]],
    ["long name in UTF-8", [file({ fileName: "€".repeat(342) })]],
    ["string size", [file({ size: "3" })]],
    ["negative size", [file({ size: -1 })]],
    ["fractional size", [file({ size: 0.5 })]],
    ["endless size", [file({ size: Infinity })]],
    ["overflowing total", [file({ size: Number.MAX_SAFE_INTEGER }), file({ id: "1", size: 1 })]],
  ] as const)("refuses: %s", (_label, list) => {
    expect(() => parseFileList(list)).toThrow("refused");
  });
});

describe("one approval", () => {
  it("is each approved file once, with its token", () => {
    const approved = new Approved<{ id: string }>();
    approved.add({ id: "a" }, "ta");
    approved.add({ id: "__proto__" }, "tp");
    expect(() => approved.take("a", "tp")).toThrow();
    expect(() => approved.take("constructor", undefined)).toThrow();
    expect(() => approved.take("toString", undefined)).toThrow();
    expect(() => approved.take(7, "ta")).toThrow();
    expect(approved.take("a", "ta").id).toBe("a");
    expect(() => approved.take("a", "ta")).toThrow();
    expect(approved.remaining).toBe(1);
    expect(approved.take("__proto__", "tp").id).toBe("__proto__");
    expect(approved.remaining).toBe(0);
  });
});

describe("refusals", () => {
  it("are limited per peer and in all", () => {
    const r = new Refusals();
    for (let i = 0; i < MAX_REFUSALS_PER_PEER; i++) expect(r.allow("greedy", 0)).toBe(true);
    expect(r.allow("greedy", 0)).toBe(false);
    for (let i = 0; i < MAX_REFUSALS - MAX_REFUSALS_PER_PEER; i++) expect(r.allow(`other ${i}`, 0)).toBe(true);
    expect(r.allow("one more", 0)).toBe(false);
    expect(r.allow("greedy", REFUSAL_WINDOW_MS)).toBe(true);
  });
});

describe("reconnecting", () => {
  const at = { now: 100_000, started: 99_000, delay: 8000, hurried: false, limitedUntil: 0 };

  it("backs off with jitter", () => {
    expect(reconnectIn(at, 0)).toBe(8000);
    expect(reconnectIn(at, 1)).toBe(10_000);
  });

  it("hurries, but not into a tight loop", () => {
    expect(reconnectIn({ ...at, hurried: true })).toBe(9000);
    expect(reconnectIn({ ...at, started: 0, hurried: true })).toBe(0);
  });

  it("never shortens the server's wait", () => {
    const limitedUntil = at.now + RATE_LIMITED_WAIT_MS;
    for (const hurried of [false, true]) {
      expect(reconnectIn({ ...at, hurried, limitedUntil })).toBe(RATE_LIMITED_WAIT_MS);
    }
  });
});

describe("the queue", () => {
  function bounded(limit: () => number) {
    const overflowed: Error[] = [];
    const stream = new StreamController<string>({
      cost: (m) => m.length + 10,
      limit,
      overflow: (e) => overflowed.push(e),
    });
    return { stream, overflowed };
  }

  it("drops everything once over the limit, and says so once", async () => {
    const { stream, overflowed } = bounded(() => 100);
    for (let i = 0; i < 5; i++) stream.add("0123456789");
    expect(overflowed).toHaveLength(0);
    stream.add("x");
    stream.add("y");
    expect(overflowed).toHaveLength(1);
    await expect(stream.readNext()).rejects.toThrow();
  });

  it("counts only what is unread", async () => {
    const { stream, overflowed } = bounded(() => 100);
    for (let i = 0; i < 50; i++) {
      stream.add("0123456789");
      expect(await stream.readNext()).toBe("0123456789");
    }
    expect(overflowed).toHaveLength(0);
  });

  it("counts empty messages", () => {
    const { stream, overflowed } = bounded(() => 100);
    for (let i = 0; i < 11; i++) stream.add("");
    expect(overflowed).toHaveLength(1);
  });

  it("follows a limit that grows", () => {
    let limit = 20;
    const { stream, overflowed } = bounded(() => limit);
    stream.add("0123456789");
    limit = 200;
    for (let i = 0; i < 9; i++) stream.add("0123456789");
    expect(overflowed).toHaveLength(0);
  });
});

// oeca: the same vectors as Oeca's src/nearby.rs (names_show_what_they_are, long_names_keep_their_extension_and_fit)
describe("received file names", () => {
  it.each([
    ["invoice\u202Efdp.exe", "invoicefdp.exe"],
    ["pay\u200Bment\u2066.pdf", "payment.pdf"],
    ["a\nb\r\tc.txt", "abc.txt"],
    ["\uFEFFx\u3164.txt", "x.txt"],
    ["  spaced.txt", "spaced.txt"],
    ["file.txt:evil.exe", "file.txt_evil.exe"],
    ["data::$DATA", "data__$DATA"],
    ["../../x.exe", ".._.._x.exe"],
    ["C:\\Windows\\x", "C__Windows_x"],
    ["CONIN$", "_CONIN$"],
    ["conout$.log", "_conout$.log"],
    ["COM\u00B9.txt", "_COM\u00B9.txt"],
    ["LPT0", "_LPT0"],
    ["con .txt", "_con .txt"],
    ["nul.tar.gz", "_.tar.gz"],
    ["CON", "_"],
    ["console.txt", "console.txt"],
    ["com10.txt", "com10.txt"],
    ["x.exe. . ", "x.exe_"],
    ["Re\u0301sume\u0301.pdf", "R\u00E9sum\u00E9.pdf"],
    ["日本語の書類.txt", "日本語の書類.txt"],
    ["\u2764\uFE0F.png", "\u2764\uFE0F.png"],
    ["", "untitled"],
    ["   ", "untitled"],
    ["..", "_"],
  ])("%j is saved and shown as %j", (untrusted, saved) => {
    expect(safeName(untrusted)).toBe(saved);
  });

  it("shortens long names before their extension, within the native limits", () => {
    for (const [untrusted, ext] of [
      ["a".repeat(300) + ".pdf", ".pdf"],
      ["日".repeat(300) + ".docx", ".docx"],
      ["\u{1F600}".repeat(200) + ".exe", ".exe"],
      ["b".repeat(400) + ".tar.gz", ".gz"],
    ]) {
      const n = safeName(untrusted!);
      expect(n.endsWith(ext!)).toBe(true);
      expect(n.length).toBeLessThanOrEqual(MAX_SAVED_NAME);
      expect(new TextEncoder().encode(n).length).toBeLessThanOrEqual(255);
    }
    // The same cut as native for a multibyte name (bytes bind first)
    expect(safeName("\u{1F600}".repeat(200) + ".exe")).toBe("\u{1F600}".repeat(62) + ".exe");
  });

  it("names the types that run code or open something elsewhere", () => {
    for (const n of ["a.EXE", "b.lnk", "c.url", "d.library-ms", "e.ps1", "f.msi", "g.dll", "h.bat", "i.cmd", "j.scr", "k.com", "l.js", "m.hta", ".exe"]) {
      expect(dangerous(n), n).toBe(true);
    }
    for (const n of ["a.pdf", "b.exe.txt", "c", "d.jpeg", "exe", "e.exe_"]) expect(dangerous(n), n).toBe(false);
    expect(dangerous(safeName("invoice\u202Efdp.exe"))).toBe(true);
    expect(dangerous(safeName("report.pdf.exe"))).toBe(true);
  });

  it("shows the real extension apart from the stem (which alone may be shortened)", () => {
    expect(nameParts("invoice.pdf.exe")).toEqual({ stem: "invoice.pdf", ext: ".exe" });
    expect(nameParts(".exe")).toEqual({ stem: ".exe", ext: "" });
    expect(nameParts("noext")).toEqual({ stem: "noext", ext: "" });
  });

  it("a received list is shown and saved under the safe names", () => {
    const [f] = parseFileList([{ id: "0", fileName: "invoice\u202Efdp.exe", size: 1, fileType: "" }]);
    expect(f!.fileName).toBe("invoicefdp.exe");
  });
});
