#!/usr/bin/env python3
"""Compare the two front ends: `--dump-ast` of the old parser against the new parser plus
lowering (`FX_FRONT=new`) over the corpus of tests/parsecheck.py (sv-tests left out unless
--only sv-tests).

  tests/frontdiff.py [--only CORPUS] [--filter REGEX] [--show N] [--diff NAME]

Per file: SAME (equal dumps), DIFF, NEWFAIL (only the new one fails), OLDFAIL (only the old
one fails), BOTHFAIL. Results go to target/frontdiff.res; --diff NAME prints one file's diff.
"""

import argparse
import concurrent.futures as cf
import difflib
import os
import re
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
from parsecheck import BIN, ROOT, jobs, limit  # noqa: E402


def dump(files, args, new):
    env = dict(os.environ)
    env.pop("FX_FRONT", None)
    if new:
        env["FX_FRONT"] = "new"
    try:
        r = subprocess.run([str(BIN), "--dump-ast", *args, *map(str, files)], capture_output=True, text=True,
                           timeout=30, preexec_fn=limit, env=env)
    except subprocess.TimeoutExpired:
        return False, "timeout"
    if r.returncode != 0:
        return False, (r.stderr.strip().splitlines() or ["?"])[-1]
    return True, r.stdout


def run(job):
    corpus, name, files, args, _ = job
    (ok_o, out_o), (ok_n, out_n) = dump(files, args, False), dump(files, args, True)
    n = f"{corpus}/{name}"
    if ok_o and ok_n:
        return n, "SAME" if out_o == out_n else "DIFF", "", (out_o, out_n)
    if ok_o:
        return n, "NEWFAIL", out_n, None
    if ok_n:
        return n, "OLDFAIL", out_o, None
    return n, "BOTHFAIL", out_n, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only")
    ap.add_argument("--filter")
    ap.add_argument("--show", type=int, default=20)
    ap.add_argument("--diff")
    ap.add_argument("-j", type=int, default=os.cpu_count() or 4)
    a = ap.parse_args()
    js = [j for j in jobs(a.only) if a.only == "sv-tests" or j[0] != "sv-tests"]
    if a.filter:
        js = [j for j in js if re.search(a.filter, f"{j[0]}/{j[1]}")]
    if a.diff:
        js = [j for j in js if f"{j[0]}/{j[1]}" == a.diff]
        for j in js:
            n, s, m, outs = run(j)
            print(s, m)
            if outs:
                sys.stdout.writelines(difflib.unified_diff(outs[0].splitlines(True), outs[1].splitlines(True), "old", "new"))
        return 0
    with cf.ThreadPoolExecutor(a.j) as ex:
        res = list(ex.map(run, js))
    (ROOT / "target/frontdiff.res").write_text("".join(f"{n}\t{s}\t{m.replace(chr(9), ' ')}\n" for n, s, m, _ in res))
    by = {}
    for n, s, _, _ in res:
        by.setdefault(n.split("/")[0], {}).setdefault(s, 0)
        by[n.split("/")[0]][s] += 1
    for c, d in by.items():
        print(f"{c}: " + ", ".join(f"{k} {v}" for k, v in sorted(d.items())))
    msgs = {}
    for n, s, m, _ in res:
        if s in ("NEWFAIL",):
            key = re.sub(r"'[^']*'", "'_'", re.sub(r"^.*?error: ", "", m))
            msgs.setdefault(key, []).append(n)
    for k, v in sorted(msgs.items(), key=lambda x: -len(x[1]))[: a.show]:
        print(f"{len(v):5}  {k}   e.g. {v[0]}")
    diffs = [n for n, s, _, _ in res if s == "DIFF"]
    for n in diffs[: a.show]:
        print(f"  diff: {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
