#!/usr/bin/env bash
# Differential test: run each testbench through Icarus (iverilog+vvp) and ferrooxide, diff stdout.
# usage: tests/difftest.sh [files...]   (default: tests/verilog/*.v)
# FX_FLAGS="--no-jit" tests/difftest.sh   runs the interpreter instead of the JIT
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release/ferrooxide"
[ -x "$bin" ] || cargo build --release -q --manifest-path "$root/Cargo.toml"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
files=("$@")
[ ${#files[@]} -eq 0 ] && files=("$root"/tests/verilog/*.v)
pass=0; fail=0; failed=()
for f in "${files[@]}"; do
  f="$(realpath "$f")"
  name="$(basename "$f" .v)"
  ( cd "$(dirname "$f")" && iverilog -g2012 -o "$tmp/$name.vvp" "$f" 2>"$tmp/$name.ierr" ) || { echo "SKIP $name (iverilog failed)"; continue; }
  ( cd "$(dirname "$f")" && vvp -n "$tmp/$name.vvp" 2>&1 | grep -v -e '\$finish called' -e '^VCD info' -e '\$stop called' > "$tmp/$name.exp" )
  ( cd "$(dirname "$f")" && timeout 60 "$bin" ${FX_FLAGS:-} "$f" > "$tmp/$name.out" 2>"$tmp/$name.err" )
  if diff -u "$tmp/$name.exp" "$tmp/$name.out" > "$tmp/$name.diff"; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed+=("$name")
    echo "FAIL $name"; head -30 "$tmp/$name.diff"; head -5 "$tmp/$name.err"
  fi
done
echo "passed $pass, failed $fail ${failed[*]:-}"
[ $fail -eq 0 ]
