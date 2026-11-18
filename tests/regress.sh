#!/usr/bin/env bash
# Run the regression suite (main and SystemVerilog lists) and compare per test with a baseline.
#   tests/regress.sh [--baseline]   (--baseline: store the results as the new baseline)
# Needs the suite at $IVTEST (default ../iverilog/ivtest); results in target/regress-*/.
# Runs the two lists one after the other (memory), each on a fresh copy (tests write files).
set -euo pipefail
cd "$(dirname "$0")/.."
src=${IVTEST:-$(realpath ../iverilog/ivtest)}
base=target/regress-baseline
out=target/regress-current
[[ ${1:-} == --baseline ]] && out=$base
rm -rf "$out" && mkdir -p "$out"
{ git rev-parse --short HEAD 2>/dev/null || echo unknown; } > "$out/commit"
for list in main sv; do
    cp -r "$src" "$out/$list"
    extra=()
    [[ $list == sv ]] && extra=(--list regress-sv.list)
    tests/ivtest.py "$out/$list" "${extra[@]}" -j 4 --show 0 --results "$out/$list.res" > "$out/$list.log" 2>&1 || true
    rm -rf "${out:?}/$list"
    grep '^[0-9]* tests:' "$out/$list.log" | sed "s/^/$list: /"
done
[[ $out == "$base" ]] && exit 0
status=0
for list in main sv; do
    diff=$(LC_ALL=C join -t$'\t' <(LC_ALL=C sort "$base/$list.res") <(LC_ALL=C sort "$out/$list.res") |
        awk -F'\t' '$2 != $4 { print "  " $1 ": " $2 " -> " $4 }')
    if [[ -n $diff ]]; then
        echo "$list: changed against the baseline ($(cat $base/commit)):"
        echo "$diff"
        grep -q -- '-> [^P]' <<< "$diff" && status=1
    fi
done
exit $status
