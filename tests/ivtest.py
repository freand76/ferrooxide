#!/usr/bin/env python3
"""Run Icarus Verilog's ivtest regression suite (regress-*.list) with ferrooxide.

usage: tests/ivtest.py IVTEST_DIR [--list regress-vlg.list] [--filter SUBSTR] [-j N] [--show N]

Run it on a copy of the ivtest directory: tests read and write files relative to it.
Pass criteria follow ivtest's vvp_reg.pl: with a gold file the output must match it line by line
(unordered=: as sorted lines; diff=a:b:skip compares two files the test wrote), otherwise the
output must contain a line with only "PASSED". Only "normal" tests are run; the summary groups
failures by their first error line.
"""
import argparse, collections, concurrent.futures as cf, os, re, resource, subprocess, sys

# per-test address-space limit: a test with a huge memory must not take the machine down
MEM_LIMIT = 2 << 30


def limit_memory():
    resource.setrlimit(resource.RLIMIT_AS, (MEM_LIMIT, MEM_LIMIT))

FX = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "ferrooxide")


def parse_list(path):
    tests = {}
    order = []
    text = open(path).read().replace("\\\n", " ")
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        f = line.split()
        name, typ = f[0], f[1]
        kind, *opts = typ.split(",")
        t = {"name": name, "kind": kind, "opts": opts, "dir": f[2] if len(f) > 2 else "",
             "top": "", "gold": "", "unordered": False, "diff": None}
        rest = f[3:]
        for r in rest:
            if r.startswith("gold="):
                t["gold"] = "gold/" + r[5:]
            elif r.startswith("unordered="):
                t["gold"] = "gold/" + r[10:]
                t["unordered"] = True
            elif r.startswith("diff="):
                p = r[5:].split(":")
                t["diff"] = (p[0], p[1], int(p[2]) if len(p) > 2 and p[2] else 0)
            elif len(rest) >= 1 and r == rest[0]:
                t["top"] = r
        if name not in tests:
            order.append(name)
        tests[name] = t
    return [tests[n] for n in order]


def run(t, root, timeout):
    src = os.path.join(".", t["dir"], t["name"] + ".v")
    args = [FX]
    plargs = []
    for o in t["opts"]:
        if o.startswith("+"):
            plargs.append(o)
        elif o.startswith("-D") or o.startswith("-I"):
            args.append(o)
        elif o.startswith("-s"):
            args += ["-s", o[2:]]
        # -g... (language generation), -T..., -p... are ignored
    if t["top"]:
        args += ["-s", t["top"]]
    args.append(src)
    args += plargs
    try:
        p = subprocess.run(args, cwd=root, capture_output=True, timeout=timeout, preexec_fn=limit_memory)
    except subprocess.TimeoutExpired:
        return "TIMEOUT", "timeout"
    out = p.stdout.decode(errors="replace")
    err = p.stderr.decode(errors="replace")
    errline = next((l for l in err.splitlines() if "error" in l), "")
    if p.returncode != 0 and not out.strip():
        if p.returncode < 0 or "panicked" in err:
            m = re.search(r"panicked at (.*)", err)
            return "CRASH", "panic: " + (m.group(1) if m else err.strip()[-200:])
        return "COMPILE", errline or err.strip().splitlines()[-1] if err.strip() else "exit %d" % p.returncode
    if t["diff"]:
        a, b, skip = t["diff"]
        try:
            la = open(os.path.join(root, a)).read().splitlines()[skip:]
            lb = open(os.path.join(root, b)).read().splitlines()[skip:]
        except OSError as e:
            return "FAIL", "diff: " + str(e)
        return ("PASS", "") if la == lb else ("FAIL", "diff files differ")
    if t["gold"]:
        try:
            gold = open(os.path.join(root, t["gold"]), errors="replace").read().replace("\r\n", "\n").splitlines()
        except OSError:
            return "FAIL", "missing gold"
        lines = out.splitlines()
        ok = sorted(gold) == sorted(lines) if t["unordered"] else gold == lines
        return ("PASS", "") if ok else ("FAIL", "output differs from gold" + (": " + errline if errline else ""))
    if any(re.match(r"^\s*passed\s*$", l, re.I) for l in out.splitlines()):
        return "PASS", ""
    return "FAIL", ("no PASSED" + (": " + errline if errline else ""))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ivtest")
    ap.add_argument("--list", default="regress-vlg.list")
    ap.add_argument("--filter", default="")
    ap.add_argument("-j", type=int, default=4)
    ap.add_argument("--timeout", type=float, default=20)
    ap.add_argument("--show", type=int, default=25, help="failure groups to show")
    ap.add_argument("--results", help="write name<TAB>status<TAB>reason lines here")
    a = ap.parse_args()
    tests = [t for t in parse_list(os.path.join(a.ivtest, a.list)) if t["kind"] == "normal" and a.filter in t["name"]]
    res = {}
    with cf.ThreadPoolExecutor(a.j) as ex:
        futs = {ex.submit(run, t, a.ivtest, a.timeout): t["name"] for t in tests}
        for f in cf.as_completed(futs):
            res[futs[f]] = f.result()
    count = collections.Counter(s for s, _ in res.values())
    print("%d tests: %s" % (len(res), ", ".join("%s %d" % kv for kv in sorted(count.items()))))
    print("pass rate %.1f%%" % (100.0 * count["PASS"] / max(1, len(res))))
    groups = collections.Counter()
    examples = {}
    for n, (s, r) in res.items():
        if s != "PASS":
            key = re.sub(r"^[^:]*:\d+: ", "", r)
            key = re.sub(r"'[^']*'", "'…'", key)[:110]
            groups[(s, key)] += 1
            examples.setdefault((s, key), n)
    for (s, k), c in groups.most_common(a.show):
        print("%5d  %-8s %s   (e.g. %s)" % (c, s, k, examples[(s, k)]))
    if a.results:
        with open(a.results, "w") as f:
            for n in sorted(res):
                f.write("%s\t%s\t%s\n" % (n, *res[n]))


if __name__ == "__main__":
    main()
