#!/usr/bin/env python3
"""Speed regression check: startup time, JavaScript/DOM benchmarks and page render times.

    python3 tools/perf_check.py            check against tools/perf-baseline.json
    python3 tools/perf_check.py --update   record this machine's numbers as the new baseline

Each number is the median of several runs. A check fails when it is more than FACTOR times
its baseline (plus a few milliseconds of slack, so tiny numbers don't flap), or when the
first window frame takes longer than the startup limit (300 ms; PERF_STARTUP_LIMIT_MS
overrides, e.g. on slow CI machines). Startup is measured only where a display (or xvfb-run)
is available. Baselines come from one machine; FACTOR leaves room for slower CI runners.
"""
import json, os, re, shutil, statistics, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BIN = os.environ.get("IRONSWIFT_BIN") or os.path.join(ROOT, "target", "release", "ironswift")
BASELINE = os.path.join(HERE, "perf-baseline.json")
FACTOR = float(os.environ.get("PERF_FACTOR", "3"))
SLACK_MS = 25
RUNS = 3
PAGES = ["tests/real-world/react.html", "tests/real-world/bootstrap.html", "tests/grid.html", "tests/tables.html"]


def env(profile):
    return dict(os.environ, IRONSWIFT_PROFILE=profile, IRONSWIFT_NO_UPDATES="1")


def render(page, profile):
    out = os.path.join(profile, "out.png")
    p = subprocess.run([BIN, os.path.join(ROOT, page), "--png", out], capture_output=True, text=True, timeout=120, env=env(profile))
    return p.stdout + p.stderr


def measure():
    results = {}
    with tempfile.TemporaryDirectory() as tmp:
        # JavaScript and DOM workloads from the benchmark page.
        runs = {}
        for _ in range(RUNS):
            log = render("tests/bench.html", tmp)
            for name, ms in re.findall(r"\[js log\] (.+?): (\d+) ms", log):
                runs.setdefault("bench: " + name, []).append(int(ms))
        # Whole-page render time (parse, style, layout, paint) for real-world pages.
        for page in PAGES:
            for _ in range(RUNS):
                m = re.search(r"([\d.]+)ms total", render(page, tmp))
                if m:
                    runs.setdefault("render: " + page, []).append(float(m.group(1)))
        for k, v in runs.items():
            results[k] = round(statistics.median(v), 1)
        # Time to the first window frame.
        xvfb = shutil.which("xvfb-run")
        if os.environ.get("DISPLAY") or xvfb:
            cmd = ([xvfb, "-a"] if not os.environ.get("DISPLAY") else []) + [BIN]
            times = []
            for i in range(5):
                e = env(os.path.join(tmp, f"start{i}"))
                e["IRONSWIFT_EXIT_AFTER_FIRST_FRAME"] = "1"
                try:
                    p = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env=e)
                except subprocess.TimeoutExpired:
                    continue
                m = re.search(r"first frame on screen after (\d+) ms", p.stdout + p.stderr)
                if m:
                    times.append(int(m.group(1)))
            if times:
                results["startup: first frame"] = statistics.median(times)
    return results


def main():
    if not os.path.exists(BIN):
        print(f"build first: {BIN} not found")
        return 2
    now = measure()
    if "--update" in sys.argv:
        with open(BASELINE, "w") as f:
            json.dump(now, f, indent=1, sort_keys=True)
            f.write("\n")
        print(f"baseline written: {len(now)} numbers")
        return 0
    base = json.load(open(BASELINE)) if os.path.exists(BASELINE) else {}
    failed = []
    print(f"{'check':<52} {'now':>9} {'baseline':>9}")
    for k in sorted(now):
        b = base.get(k)
        bad = b is not None and now[k] > b * FACTOR + SLACK_MS and not k.startswith("startup")
        print(f"{k:<52} {now[k]:>9} {b if b is not None else '-':>9}{'  SLOWER' if bad else ''}")
        if bad:
            failed.append(k)
    limit = float(os.environ.get("PERF_STARTUP_LIMIT_MS", "300"))
    s = now.get("startup: first frame")
    if s is not None and s > limit:
        failed.append(f"startup {s} ms > {limit:.0f} ms")
    missing = [k for k in base if k not in now and not k.startswith("startup")]
    if missing:
        failed.append("missing results: " + ", ".join(missing))
    if failed:
        print("\nSpeed regressions:\n  " + "\n  ".join(failed))
        return 1
    print("\nNo speed regressions.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
