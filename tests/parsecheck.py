#!/usr/bin/env python3
"""Parse-only check of the new parser over a corpus (`ferrooxide --parse-only`).

Corpora: sv-tests (~/projects/sv-tests, IEEE 1800 conformance; a test marked
`:should_fail_because:` with type "parsing" must fail, others must parse), the regression
suite sources, this repository's tests, and real designs (read only).

  tests/parsecheck.py [--baseline] [--show N] [--only CORPUS] [--filter REGEX]

Results go to target/parsecheck-current.res (name<TAB>status<TAB>message); --baseline stores
them as target/parsecheck-baseline.res. Without --baseline, files whose status got worse
against the baseline are listed and the exit status is 1.
"""

import argparse
import concurrent.futures as cf
import os
import re
import resource
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOME = Path.home() / "projects"
BIN = ROOT / "target/release/ferrooxide"
DESIGNS = ["picorv32", "TetrIC", "TetrIC_2025", "NanoMig", "nestang", "hdmi", "Axi4-lite",
           "FaKeyboard", "rv32e", "tangnano20k-picorv32", "picorv32_tang_nano_20k",
           "gowin_fpga_sim_models", "W25Q64JVxxIM", "monty-pythons-flying-circuits", "kalkon"]


def limit():
    resource.setrlimit(resource.RLIMIT_AS, (1 << 30, 1 << 30))


def meta(text, key):
    m = re.search(r"^:" + key + r":\s*(.*)$", text, re.M)
    return m.group(1).strip() if m else None


def jobs(only):
    """(corpus, name, files, args, expect) with expect 'pass', 'fail' or None (either)."""
    out = []
    sv = HOME / "sv-tests/tests"
    if sv.is_dir() and only in (None, "sv-tests"):
        for f in sorted(sv.rglob("*.sv")):
            rel = f.relative_to(sv)
            if rel.parts[0] in ("uvm", "testbenches"):
                continue
            text = f.read_text(errors="replace")
            args = []
            for d in (meta(text, "incdirs") or "").split():
                args += ["-I", str((HOME / "sv-tests" / d))]
            for d in (meta(text, "defines") or "").split():
                args += ["-D", d]
            files = [f] + [HOME / "sv-tests" / x for x in (meta(text, "files") or "").split()]
            fail = meta(text, "should_fail_because")
            typ = meta(text, "type") or ""
            expect = "pass"
            if fail is not None:
                expect = "fail" if ("parsing" in typ or "preprocessing" in typ) else None
            out.append(("sv-tests", str(rel), files, args, expect))
    iv = HOME / "iverilog/ivtest"
    if iv.is_dir() and only in (None, "ivtest"):
        for f in sorted(list(iv.rglob("*.v")) + list(iv.rglob("*.sv"))):
            out.append(("ivtest", str(f.relative_to(iv)), [f], ["-I", str(f.parent), "-I", str(iv / "ivltests")], None))
    if only in (None, "own"):
        for d in ["tests/verilog", "tests/suite", "crates/ferrooxide-front/tests/parse", "bench"]:
            for f in sorted(list((ROOT / d).rglob("*.v")) + list((ROOT / d).rglob("*.sv"))):
                out.append(("own", str(f.relative_to(ROOT)), [f], ["-I", str(f.parent)], None))
    if only in (None, "designs"):
        for d in DESIGNS:
            p = HOME / d
            if not p.is_dir():
                continue
            for f in sorted(list(p.rglob("*.v")) + list(p.rglob("*.sv"))):
                if ".git" in f.parts or any(x.startswith(".venv") or x == "site-packages" for x in f.parts):
                    continue
                out.append(("designs", str(f.relative_to(HOME)), [f], ["-I", str(f.parent)], None))
    return out


def run(job):
    corpus, name, files, args, expect = job
    try:
        r = subprocess.run([str(BIN), "--parse-only", *args, *map(str, files)], capture_output=True, text=True,
                           timeout=30, preexec_fn=limit)
        ok = r.returncode == 0
        msg = "" if ok else (r.stderr.strip().splitlines() or ["?"])[-1]
        if r.returncode < 0 or "panicked" in r.stderr:
            return f"{corpus}/{name}", "CRASH", (r.stderr.strip().splitlines() or ["?"])[-1]
    except subprocess.TimeoutExpired:
        return f"{corpus}/{name}", "TIMEOUT", ""
    if "uvm_macros.svh" in msg:
        return f"{corpus}/{name}", "SKIP", "needs UVM"
    if expect is None and corpus == "sv-tests":
        return f"{corpus}/{name}", "PASS" if ok else "XFAIL", msg.replace("\t", " ")
    if expect == "fail":
        return f"{corpus}/{name}", "PASS" if not ok else "FAIL", "" if not ok else "parsed, should fail"
    return f"{corpus}/{name}", "PASS" if ok else "FAIL", msg.replace("\t", " ")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", action="store_true")
    ap.add_argument("--show", type=int, default=20)
    ap.add_argument("--only")
    ap.add_argument("--filter")
    ap.add_argument("-j", type=int, default=os.cpu_count() or 4)
    a = ap.parse_args()
    js = jobs(a.only)
    if a.filter:
        js = [j for j in js if re.search(a.filter, f"{j[0]}/{j[1]}")]
    with cf.ThreadPoolExecutor(a.j) as ex:
        res = list(ex.map(run, js))
    out = ROOT / "target" / ("parsecheck-baseline.res" if a.baseline else "parsecheck-current.res")
    out.write_text("".join(f"{n}\t{s}\t{m}\n" for n, s, m in res))
    by = {}
    for n, s, m in res:
        c = n.split("/")[0]
        by.setdefault(c, {}).setdefault(s, 0)
        by[c][s] += 1
    for c, d in by.items():
        print(f"{c}: " + ", ".join(f"{k} {v}" for k, v in sorted(d.items())))
    fails = [(n, s, m) for n, s, m in res if s != "PASS"]
    # most frequent messages
    msgs = {}
    for n, s, m in fails:
        key = re.sub(r"^.*?error: ", "", m)
        key = re.sub(r"'[^']*'", "'_'", key)
        msgs.setdefault(key, []).append(n)
    for k, v in sorted(msgs.items(), key=lambda x: -len(x[1]))[: a.show]:
        print(f"{len(v):5}  {k}   e.g. {v[0]}")
    base = ROOT / "target/parsecheck-baseline.res"
    if not a.baseline and base.exists():
        old = {l.split("\t")[0]: l.split("\t")[1] for l in base.read_text().splitlines() if l}
        worse = [(n, old[n], s) for n, s, _ in res if n in old and old[n] == "PASS" and s != "PASS"]
        better = sum(1 for n, s, _ in res if n in old and old[n] != "PASS" and s == "PASS")
        print(f"against the baseline: {better} better, {len(worse)} worse")
        for n, o, s in worse[:50]:
            print(f"  worse: {n}: {o} -> {s}")
        return 1 if worse else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
