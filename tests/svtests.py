#!/usr/bin/env python3
"""Run chipsalliance sv-tests (~/projects/sv-tests, or $SV_TESTS) with ferrooxide, the way
the sv-tests ferrooxide runner does, and compare the results with a baseline.

  tests/svtests.py [--baseline] [--tags] [--show N] [--filter REGEX] [-j N]

$FX_BIN selects another ferrooxide binary (default target/release/ferrooxide).

Each test runs in the most complete mode its `:type:` allows (simulation, then elaboration,
then parsing): simulation is a full run (once per `:simvariants:` entry, whose `:assert:`
output lines must hold), elaboration `--elab-only`, parsing `--parse-only`. A test passes when
ferrooxide succeeds, or fails for a test marked `:should_fail_because:`.

Results go to target/svtests-current.res (name<TAB>status<TAB>mode<TAB>tags<TAB>message);
--baseline stores them as target/svtests-baseline.res. Statuses: PASS, REJECT (valid code
rejected), ACCEPT (invalid code accepted), ASSERT (an `:assert:` line failed), CRASH,
TIMEOUT. Without --baseline, tests that got worse against the baseline are listed and the
exit status is 1. --tags prints the pass rate per tag.
"""

import argparse
import concurrent.futures as cf
import json
import os
import re
import resource
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SV = Path(os.environ.get("SV_TESTS", Path.home() / "projects/sv-tests"))
BIN = Path(os.environ.get("FX_BIN", ROOT / "target/release/ferrooxide"))
MODES = ["simulation", "simulation_without_run", "elaboration", "parsing", "preprocessing"]
EXTS = (".v", ".sv", ".vh", ".svh")


def limit():
    # 2 GB of address space per run (ferrooxide's own cap is larger)
    resource.setrlimit(resource.RLIMIT_AS, (2 << 30, 2 << 30))


def params(path):
    """The `:key: value` metadata of a test (as sv-tests reads it), with its defaults."""
    p = {}
    for line in path.read_text(errors="replace").splitlines():
        m = re.match(r"^:([a-zA-Z_-]+):\s*(.+)", line)
        if m:
            p[m.group(1).lower()] = m.group(2).strip()
    p.setdefault("files", str(path))
    p.setdefault("incdirs", str(path.parent))
    p.setdefault("type", "parsing elaboration")
    p.setdefault("timeout", "30")
    return p


def jobs():
    libs = json.loads((SV / "conf/runners/libs.json").read_text())
    out = []
    for f in sorted((SV / "tests").rglob("*.sv")):
        p = params(f)
        mode = next((m for m in MODES if m in p["type"].split()), None)
        if mode is None:
            continue
        cr = p.get("compatible-runners", "all").split()
        if "all" not in cr and "ferrooxide" not in cr:
            continue
        tags = p.get("tags", "").split()
        files = [x if os.path.isabs(x) else str(SV / x) for x in p["files"].split()]
        incdirs = [x if os.path.isabs(x) else str(SV / "tests" / x) for x in p["incdirs"].split()]
        for k, lib in libs.items():
            if k in tags:
                files = [str(SV / "third_party" / x) for x in lib["files"]] + files
                incdirs = [str(SV / "third_party" / x) for x in lib["incdirs"]] + incdirs
        files = [x for x in files if os.path.splitext(x)[1] in EXTS]
        cmd = [str(BIN), "-q"]
        if mode in ("preprocessing", "parsing"):
            cmd.append("--parse-only")
        elif mode in ("elaboration", "simulation_without_run"):
            cmd.append("--elab-only")
        if p.get("top_module"):
            cmd += ["-s", p["top_module"]]
        cmd += ["-I" + d for d in incdirs]
        # UVM without its DPI C code, as the runner does
        cmd.append("-DUVM_NO_DPI")
        cmd += ["-D" + d for d in p.get("defines", "").split()]
        cmd += files
        variants = p.get("simvariants", "").split() if mode == "simulation" else []
        fail = "should_fail_because" in p or p.get("should_fail") == "1"
        name = str(f.relative_to(SV / "tests"))
        out.append((name, mode, tags, cmd, variants, fail, int(p["timeout"])))
    return out


def asserts_hold(log):
    """sv-tests' check of a simulation log: every `:assert: <python expression>` is true."""
    for line in log.splitlines():
        m = re.search(r":([a-z]+):(.*)", line.strip())
        if m and m.group(1) == "assert":
            try:
                if not eval(m.group(2), {}):
                    return False
            except Exception:
                return False
    return True


def run(job):
    name, mode, tags, cmd, variants, fail, timeout = job
    log = ""
    rc = 0
    with tempfile.TemporaryDirectory() as tmp:
        for extra in ([[v] for v in variants] or [[]]):
            try:
                r = subprocess.run(cmd + extra, cwd=tmp, capture_output=True, timeout=timeout, preexec_fn=limit)
                out = r.stdout.decode("utf-8", "ignore") + r.stderr.decode("utf-8", "ignore")
                log += out
                rc = r.returncode
            except subprocess.TimeoutExpired:
                return name, "TIMEOUT", mode, tags, f"> {timeout} s"
            if rc != 0:
                break
    errs = [l for l in log.splitlines() if "error" in l or "panicked" in l]
    msg = (errs or log.strip().splitlines() or [""])[0].replace("\t", " ")[:300]
    if rc < 0 or rc >= 126 or "panicked" in log:
        return name, "CRASH", mode, tags, msg
    if fail:
        return name, ("PASS" if rc != 0 else "ACCEPT"), mode, tags, ""
    if rc != 0:
        return name, "REJECT", mode, tags, msg
    if mode == "simulation" and not asserts_hold(log):
        return name, "ASSERT", mode, tags, next((l for l in log.splitlines() if ":assert:" in l), "")
    return name, "PASS", mode, tags, ""


def key(msg):
    """An error message with names and numbers left out (for grouping)."""
    k = re.sub(r"^.*?error: ", "", msg)
    k = re.sub(r"'[^']*'", "'…'", k)
    return re.sub(r"\b\d+\b", "N", k)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", action="store_true")
    ap.add_argument("--tags", action="store_true")
    ap.add_argument("--show", type=int, default=20)
    ap.add_argument("--filter")
    ap.add_argument("-j", type=int, default=4)
    a = ap.parse_args()
    if not (SV / "tests").is_dir():
        print(f"no sv-tests at {SV} (set SV_TESTS)", file=sys.stderr)
        return 2
    js = jobs()
    if a.filter:
        js = [j for j in js if re.search(a.filter, j[0])]
    with cf.ThreadPoolExecutor(a.j) as ex:
        res = list(ex.map(run, js))
    out = ROOT / "target" / ("svtests-baseline.res" if a.baseline else "svtests-current.res")
    out.write_text("".join(f"{n}\t{s}\t{m}\t{' '.join(t)}\t{msg}\n" for n, s, m, t, msg in res))
    npass = sum(1 for r in res if r[1] == "PASS")
    counts = {}
    for r in res:
        counts[r[1]] = counts.get(r[1], 0) + 1
    print(f"{npass} of {len(res)} tests pass ({100 * npass / max(len(res), 1):.1f}%): "
          + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    if a.tags:
        by = {}
        for n, s, _, tags, _ in res:
            for t in tags or ["(none)"]:
                d = by.setdefault(t, [0, 0])
                d[0] += s == "PASS"
                d[1] += 1
        for t, (p, n) in sorted(by.items(), key=lambda x: (x[1][0] - x[1][1], x[0])):
            if p < n:
                print(f"  {t:30} {p:5} of {n:5}")
    msgs = {}
    for n, s, _, _, m in res:
        if s in ("REJECT", "CRASH", "TIMEOUT", "ASSERT"):
            msgs.setdefault(f"{s}: {key(m)}", []).append(n)
    for k, v in sorted(msgs.items(), key=lambda x: -len(x[1]))[: a.show]:
        print(f"{len(v):5}  {k}   e.g. {v[0]}")
    base = ROOT / "target/svtests-baseline.res"
    if not a.baseline and base.exists():
        old = {l.split("\t")[0]: l.split("\t")[1] for l in base.read_text().splitlines() if l}
        worse = [(n, old[n], s) for n, s, *_ in res if old.get(n) == "PASS" and s != "PASS"]
        better = sum(1 for n, s, *_ in res if n in old and old[n] != "PASS" and s == "PASS")
        print(f"against the baseline: {better} better, {len(worse)} worse")
        for n, o, s in worse[:50]:
            print(f"  worse: {n}: {o} -> {s}")
        return 1 if worse else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
