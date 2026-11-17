#!/bin/sh
# Profile a JavaScript file running in Ironswift's copy of Boa.
#
#   tools/profile_js.sh script.js [more.js ...]
#
# With `perf` installed (Linux): records call stacks and writes profile.svg, a flame graph
# (needs `cargo install inferno` once). Without perf: runs under valgrind's callgrind and prints
# the functions that take the most time (much slower; use a short script).
# Real site scripts: save them from the page (or from tests/real-world) and pass them here.
set -e
cd "$(dirname "$0")/.."
[ $# -gt 0 ] || { echo "usage: $0 script.js [more.js ...]"; exit 1; }
# Keep symbols and frame pointers so the call stacks are readable.
RUSTFLAGS="${RUSTFLAGS} -C force-frame-pointers=yes" CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
  cargo build --release --example jsbench --target-dir target/profile
BIN=target/profile/release/examples/jsbench
if command -v perf >/dev/null 2>&1; then
  perf record -F 999 -g -o perf.data "$BIN" "$@"
  if command -v inferno-collapse-perf >/dev/null 2>&1; then
    perf script -i perf.data | inferno-collapse-perf | inferno-flamegraph > profile.svg
    echo "Flame graph: profile.svg"
  else
    perf report -i perf.data --no-children --percent-limit 1 --stdio | head -80
    echo "(cargo install inferno for a flame graph)"
  fi
elif command -v valgrind >/dev/null 2>&1; then
  valgrind --tool=callgrind --callgrind-out-file=callgrind.out "$BIN" "$@"
  callgrind_annotate --inclusive=no callgrind.out 2>/dev/null | head -60
  echo "Full data: callgrind.out (open with kcachegrind)"
else
  echo "Install perf (linux-tools) or valgrind first."; exit 1
fi
