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
  # optional plusargs for both simulators: a line "// args: +a=1 +b"
  args="$(sed -n 's|^// args:||p' "$f" | head -1)"
  # optional Icarus flags: a line "// iverilog: -g2005" (default -g2012)
  iflags="$(sed -n 's|^// iverilog:||p' "$f" | head -1)"
  # each simulator runs in its own directory: tests may write (and read back) files
  rm -rf "$tmp/iv" "$tmp/fx"; mkdir -p "$tmp/iv" "$tmp/fx"
  ( cd "$(dirname "$f")" && iverilog ${iflags:--g2012} -o "$tmp/$name.vvp" "$f" 2>"$tmp/$name.ierr" ) || { echo "SKIP $name (iverilog failed)"; continue; }
  ( cd "$tmp/iv" && vvp -n "$tmp/$name.vvp" $args 2>&1 | grep -v -e '\$finish called' -e '\$stop called' > "$tmp/$name.exp" )
  ( cd "$tmp/fx" && timeout 60 "$bin" ${FX_FLAGS:-} "$f" $args 2>"$tmp/$name.err" | grep -v -e '\$finish called' -e '\$stop called' > "$tmp/$name.out" )
  if diff -u "$tmp/$name.exp" "$tmp/$name.out" > "$tmp/$name.diff"; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed+=("$name")
    echo "FAIL $name"; head -30 "$tmp/$name.diff"; head -5 "$tmp/$name.err"
  fi
done
echo "passed $pass, failed $fail ${failed[*]:-}"
[ $fail -eq 0 ]
