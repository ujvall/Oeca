"""Regenerates section 6 of THIRD-PARTY-NOTICES.txt (Nearby) from the locked dependency graph.

    python tools/nearby_notices.py          rewrite section 6 (everything from its heading to the end)
    python tools/nearby_notices.py --check  exit 1 if the file is not what this would write
    python tools/nearby_notices.py --report list the crates and anything missing, write nothing

Section 6 lists every crate compiled into Oeca.exe (normal dependencies, for
x86_64-pc-windows-msvc) that sections 1-5 do not, with its own license files
(and NOTICE files, which Apache-2.0 requires to be passed on), as section 5
does. Runs `cargo metadata`, which fetches any crate source not present yet
(the release workflow runs this as a check, after the tests).
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NOTICES = ROOT / "THIRD-PARTY-NOTICES.txt"
HEADING = "6. Oeca Hop: the LocalSend core and its Rust dependencies (compiled into Oeca.exe)"
RULE = "=" * 78
LINE = "-" * 78
# Crates published without any license file: the same project's text, from a
# crate of the same repository that ships it (said so where it is reproduced)
SAME_PROJECT = {
    "asn1-rs-impl": "asn1-rs",  # github.com/rusticata/asn1-rs
    "webrtc": "webrtc-util",  # github.com/webrtc-rs/webrtc
    "yasna": "yasna",  # github.com/qnighy/yasna.rs (a later version)
}
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)", re.I)


def metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", "x86_64-pc-windows-msvc"],
        cwd=ROOT, capture_output=True, text=True, encoding="utf-8", check=True,
    ).stdout
    return json.loads(out)


def shipped(meta):
    """Packages reachable from Oeca by normal (not dev, not build) dependencies."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    packages = {p["id"]: p for p in meta["packages"]}
    root = meta["resolve"]["root"]
    seen, todo = set(), [root]
    while todo:
        i = todo.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] is None for k in d["dep_kinds"]):
                todo.append(d["pkg"])
    return [packages[i] for i in seen if i != root]


def listed_before(text):
    """name version pairs already listed in sections 1-5."""
    head = text.split(f"{RULE}\n{HEADING}", 1)[0]
    pairs = set(re.findall(r"^  ([A-Za-z0-9_-]+) (\d[^\s:]*)", head, re.M))
    return pairs


def used(expr):
    """The license relied on: MIT where offered, otherwise the expression itself."""
    expr = (expr or "").replace("/", " OR ")
    options = [o.strip(" ()") for o in re.split(r"\bOR\b", expr)]
    if "AND" not in expr and "MIT" in options:
        return "MIT"
    return expr.strip()


def files_for(pkg, license_used):
    folder = Path(pkg["manifest_path"]).parent
    found = sorted(f for f in folder.iterdir() if f.is_file() and LICENSE_FILE.match(f.name))
    notices = [f for f in found if f.name.upper().startswith("NOTICE")]
    if license_used == "MIT":
        mit = [f for f in found if "MIT" in f.name.upper()]
        if mit:
            return mit + notices
        single = [f for f in found if f.name.upper() in ("LICENSE", "LICENSE.MD", "LICENSE.TXT", "LICENCE", "COPYING")]
        return (single or found) + notices
    return found


def build(text):
    meta = metadata()
    before = listed_before(text)
    crates, missing = [], []
    for p in sorted(shipped(meta), key=lambda p: (p["name"], p["version"])):
        if (p["name"], p["version"]) in before:
            continue
        if p["name"] == "localsend":  # the vendored LocalSend core is in the heading below
            continue
        lic = used(p.get("license"))
        files = files_for(p, lic)
        if not files and p["name"] in SAME_PROJECT:
            sibling = max(
                (q for q in meta["packages"] if q["name"] == SAME_PROJECT[p["name"]] and q["id"] != p["id"]),
                key=lambda q: q["version"],
            )
            files = [(f, f"{sibling['name']} {sibling['version']}") for f in files_for(sibling, lic)]
        if not files or not p.get("license"):
            missing.append(f"{p['name']} {p['version']}: license={p.get('license')!r}, files={[f.name for f in files]}")
        crates.append((p, lic, files))
    localsend = next(p for p in meta["packages"] if p["name"] == "localsend")
    ls_license = (Path(localsend["manifest_path"]).parent / "LICENSE").read_text(encoding="utf-8")
    out = [RULE, HEADING, RULE, "", """Oeca Hop uses the LocalSend core (https://github.com/localsend/localsend,
packages/core, commit 26cc8f997aa3f7ad789c12500a8b7206cfbb3ddc), included
as a modified copy under vendor/localsend-core in the Oeca source: Oeca
changed src/webrtc/signaling.rs and src/webrtc/webrtc.rs (each says so; the
changes are in vendor/localsend-core/OECA-PATCHES.diff).
Copyright (c) LocalSend contributors. Licensed under the Apache License,
Version 2.0 (vendor/localsend-core/LICENSE, reproduced below); you may not use
it except in compliance with that License. It is distributed on an "AS IS"
BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.

The crates below are compiled into Oeca.exe for Oeca Hop (and were not already
listed above). For crates offered under several licenses, the one Oeca relies
on is named; each crate's own license files (and NOTICE files) follow, with
its copyright notices. webpki-roots carries Mozilla's CA certificate list
(CDLA-Permissive-2.0); ring is licensed under Apache-2.0 AND ISC. dtls is
included as a modified copy under vendor/dtls in the Oeca source (it negotiates
X25519 only; the change is in vendor/dtls/OECA-VENDOR.txt and OECA-PATCHES.diff).
Generated by tools/nearby_notices.py from Cargo.lock.

  crate version: license expression -> license used"""]
    for p, lic, _ in crates:
        out.append(f"  {p['name']} {p['version']}: {p.get('license')} -> {lic}")
    out += ["", LINE, "localsend (vendored, modified) (Apache-2.0): LICENSE", LINE, ls_license.rstrip()]
    for p, lic, files in crates:
        for f in files:
            f, borrowed = f if isinstance(f, tuple) else (f, None)
            body = f.read_text(encoding="utf-8", errors="replace").replace("\r\n", "\n").rstrip()
            label = f"{p['name']} {p['version']} ({lic}): {f.name}"
            if borrowed:
                label += f" (the crate ships none; the same project's, from {borrowed})"
            out += ["", LINE, label, LINE, body]
    section = "\n".join(out) + "\n"
    head = text.split(f"{RULE}\n{HEADING}", 1)[0]
    return head + section, crates, missing


def main():
    raw = NOTICES.read_bytes().decode("utf-8")
    nl = "\r\n" if "\r\n" in raw else "\n"
    text = raw.replace("\r\n", "\n")
    new, crates, missing = build(text)
    if "--report" in sys.argv:
        print(f"{len(crates)} crates in section 6")
        for m in missing:
            print("MISSING", m)
        return 0
    if "--check" in sys.argv:
        ok = new == text
        print("THIRD-PARTY-NOTICES.txt section 6 is " + ("current" if ok else "out of date: run tools/nearby_notices.py"))
        return 0 if ok else 1
    NOTICES.write_bytes(new.replace("\n", nl).encode("utf-8"))
    print(f"section 6: {len(crates)} crates; missing license files: {len(missing)}")
    for m in missing:
        print("MISSING", m)
    return 0


if __name__ == "__main__":
    sys.exit(main())
