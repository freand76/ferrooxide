#!/usr/bin/env bash
# Our own test suite: tests/suite/<area>/<name>.v, run in all three modes (default, --no-jit,
# --no-fuse). No other simulator needed.
#
# A test passes when
#   - <name>.expected exists: stdout equals it exactly;
#   - otherwise (self-checking): stdout has a line "PASSED" and no line starting with "FAILED";
#     a test with a nonzero `// exit:` (an error test) only needs that exit status.
# Header lines in the test:
#   // args: +a=1 +b      plusargs
#   // flags: -g2005      extra ferrooxide options
#   // exit: 1            expected exit status (default 0), e.g. for tests of errors
#   // expected           compare with <name>.expected (lets --bless create it)
#
# usage: tests/suite.sh [-v] [--bless] [files or area dirs...]   (default: all of tests/suite)
#   -v       show the output of failing tests
#   --bless  write <name>.expected from the default mode's output where it is missing or
#            differs (review the diff before committing!)
set -u
ulimit -v 3000000
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="$root/target/release/ferrooxide"
[ -x "$bin" ] || cargo build --release -q --manifest-path "$root/Cargo.toml"
verbose=0; bless=0; args=()
for a in "$@"; do
  case "$a" in
    -v) verbose=1 ;;
    --bless) bless=1 ;;
    *) args+=("$a") ;;
  esac
done
[ ${#args[@]} -eq 0 ] && args=("$root/tests/suite")
files=()
for a in "${args[@]}"; do
  if [ -d "$a" ]; then
    while IFS= read -r f; do files+=("$f"); done < <(find "$a" -name '*.v' | LC_ALL=C sort)
  else
    files+=("$a")
  fi
done
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
pass=0; fail=0; failed=()
for f in "${files[@]}"; do
  f="$(realpath "$f")"
  rel="${f#$root/tests/suite/}"; rel="${rel%.v}"
  exp="${f%.v}.expected"
  pargs="$(sed -n 's|^// args:||p' "$f" | head -1)"
  flags="$(sed -n 's|^// flags:||p' "$f" | head -1)"
  want_rc="$(sed -n 's|^// exit: *||p' "$f" | head -1)"; want_rc="${want_rc:-0}"
  # compared with <name>.expected: when that file exists or the test says "// expected"
  by_output=0
  { [ -f "$exp" ] || grep -q '^// expected' "$f"; } && by_output=1
  ok=1; why=""
  for mode in "" --no-jit --no-fuse; do
    # in a copy of the test's directory (tests may write files and read data files next to
    # them), with a relative path: it appears in messages
    rm -rf "$tmp/run"; cp -r "$(dirname "$f")" "$tmp/run"
    ( cd "$tmp/run" && timeout 60 "$bin" -q $mode $flags "$(basename "$f")" $pargs ) >"$tmp/out" 2>"$tmp/err"
    rc=$?
    if [ "$rc" != "$want_rc" ]; then
      ok=0; why="exit $rc (want $want_rc) ${mode:-default}"; break
    fi
    if [ $by_output = 1 ]; then
      if [ ! -f "$exp" ] || ! cmp -s "$exp" "$tmp/out"; then
        if [ $bless = 1 ] && [ -z "$mode" ]; then
          cp "$tmp/out" "$exp"; echo "BLESSED $rel"
          continue
        fi
        ok=0; why="output differs ${mode:-default}"
        [ $verbose = 1 ] && diff -u "$exp" "$tmp/out" | head -30
        break
      fi
    elif [ "$want_rc" = 0 ]; then
      if ! grep -qx 'PASSED' "$tmp/out" || grep -q '^FAILED' "$tmp/out"; then
        ok=0; why="no PASSED or a FAILED line ${mode:-default}"
        [ $verbose = 1 ] && head -30 "$tmp/out"
        break
      fi
    fi
  done
  if [ $ok = 1 ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed+=("$rel")
    echo "FAIL $rel: $why"
    [ $verbose = 1 ] && head -5 "$tmp/err"
  fi
done
echo "suite: passed $pass, failed $fail ${failed[*]:-}"
[ $fail -eq 0 ]
