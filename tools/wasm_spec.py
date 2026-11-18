#!/usr/bin/env python3
"""Run the WebAssembly spec tests (.wast files) through Ironswift's JavaScript API.

    python3 tools/wasm_spec.py /path/to/wpt /path/to/spec/test/core

Builds tools/wast2wpt, turns every .wast file (and those in the proposal folders under it:
bulk-memory, simd, gc…) into a testharness.js page under WPT's wasm/spec-tests/, and runs the
pages with tools/wpt.py (so WPT_TIMEOUT and WPT_DETAILS work as there). Each step of a .wast
file (a module, an assert_return…) is one subtest.

WPT's own copy of the suite (wasm/core) checks results with wrapper modules that need the GC
proposal, so engines without GC fail nearly all of it; these pages check results in script.
What script can't express (v128 values, NaN payloads, GC references) is skipped; wast2wpt
prints how many steps each file skipped.

The spec tests: git clone --depth 1 https://github.com/WebAssembly/spec (test/core).
"""
import os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    wpt, core = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    tool_dir = os.path.join(HERE, "wast2wpt")
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=tool_dir, check=True)
    tool = os.path.join(tool_dir, "target", "release", "wast2wpt")
    out_root = os.path.join(wpt, "wasm", "spec-tests")
    dirs = []
    for sub in [""] + sorted(d for d in os.listdir(core) if os.path.isdir(os.path.join(core, d))):
        src = os.path.join(core, sub)
        files = sorted(os.path.join(src, f) for f in os.listdir(src) if f.endswith(".wast"))
        if not files:
            continue
        rel = "wasm/spec-tests/core" + ("/" + sub if sub else "")
        out = subprocess.run([tool, os.path.join(wpt, rel)] + files, capture_output=True, text=True)
        sys.stderr.write(out.stderr)
        dirs.append(rel)
    os.makedirs(out_root, exist_ok=True)
    return subprocess.run([sys.executable, os.path.join(HERE, "wpt.py"), wpt] + dirs).returncode


if __name__ == "__main__":
    sys.exit(main())
