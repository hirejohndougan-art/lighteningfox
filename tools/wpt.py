#!/usr/bin/env python3
"""Run a subset of the Web Platform Tests (wpt.fyi) against Ironswift.

    python3 tools/wpt.py /path/to/wpt dom/nodes css/css-flexbox ...

Script tests (testharness.js) report pass/fail per subtest. Reference tests (<link rel=match>)
pass when the test page and its reference render to identical pixels (800x600, like WPT).
Needs a WPT checkout; the script swaps in tools/testharnessreport.js and serves the files on
localhost.
"""
import concurrent.futures, functools, http.server, json, os, re, shutil, subprocess, sys, tempfile, threading

HERE = os.path.dirname(os.path.abspath(__file__))
BROWSER = os.environ.get("IRONSWIFT_BIN") or os.path.join(HERE, "..", "target", "release", "ironswift")
STATUS = {0: "PASS", 1: "FAIL", 2: "TIMEOUT", 3: "NOTRUN", 4: "PRECONDITION_FAILED"}


def serve(root):
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass
    handler = functools.partial(Quiet, directory=root)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv.server_address[1]


def render(url, png, profile):
    env = dict(os.environ, IRONSWIFT_PROFILE=profile, NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
    try:
        p = subprocess.run([BROWSER, "--width", "800", "--png", png, url], capture_output=True, text=True, timeout=30, env=env)
        return p.stdout + p.stderr, p.returncode
    except subprocess.TimeoutExpired:
        return "", "timeout"


def script_test(base, rel, tmp):
    log, rc = render(f"{base}/{rel}", os.path.join(tmp, "x.png"), os.path.join(tmp, "prof"))
    m = re.search(r"WPT-RESULT (\{.*\})", log)
    if not m:
        return rel, None, "no result" if rc == 0 else f"exit {rc}"
    r = json.loads(m.group(1))
    tests = [(t["name"], STATUS.get(t["status"], "?")) for t in r["tests"]]
    if r["status"] != 0 and not tests:
        # The harness itself failed (usually in setup): count the file as one failure.
        tests = [("(harness: " + r["message"][:80] + ")", "FAIL")]
    return rel, tests, None


def load_rgb(path):
    from PIL import Image
    im = Image.open(path).convert("RGB")
    canvas = Image.new("RGB", (800, 600), im.getpixel((0, 0)))
    canvas.paste(im.crop((0, 0, 800, min(600, im.height))), (0, 0))
    return canvas


def ref_test(base, rel, ref, tmp):
    a, b = os.path.join(tmp, "t.png"), os.path.join(tmp, "r.png")
    _, rc1 = render(f"{base}/{rel}", a, os.path.join(tmp, "prof"))
    _, rc2 = render(f"{base}/{ref}", b, os.path.join(tmp, "prof"))
    if rc1 != 0 or rc2 != 0:
        return rel, False, f"exit {rc1}/{rc2}"
    from PIL import ImageChops
    same = ImageChops.difference(load_rgb(a), load_rgb(b)).getbbox() is None
    return rel, same, None


def classify(root, rel):
    text = open(os.path.join(root, rel), encoding="utf-8", errors="replace").read()
    m = re.search(r'<link[^>]+rel=["\']?match["\']?[^>]*>', text, re.I)
    if m:
        href = re.search(r'href=["\']?([^"\'\s>]+)', m.group(0))
        if href:
            ref = href.group(1)
            ref = ref.lstrip("/") if ref.startswith("/") else os.path.normpath(os.path.join(os.path.dirname(rel), ref))
            return "ref", ref
    if "testharness.js" in text:
        return "script", None
    return None, None


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    root, dirs = os.path.abspath(sys.argv[1]), sys.argv[2:]
    report = os.path.join(root, "resources", "testharnessreport.js")
    if not os.path.exists(report + ".orig"):
        shutil.copy(report, report + ".orig")
    shutil.copy(os.path.join(HERE, "testharnessreport.js"), report)
    base = f"http://127.0.0.1:{serve(root)}"
    jobs = []
    for d in dirs:
        for name in sorted(os.listdir(os.path.join(root, d))):
            if not re.search(r"\.(html?|xhtml)$", name) or "-ref." in name or name.endswith("-notref.html"):
                continue
            rel = f"{d}/{name}"
            kind, ref = classify(root, rel)
            if kind:
                jobs.append((d, kind, rel, ref))
    results = {}
    def run(job):
        d, kind, rel, ref = job
        with tempfile.TemporaryDirectory() as tmp:
            return d, kind, (script_test(base, rel, tmp) if kind == "script" else ref_test(base, rel, ref, tmp))
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 2) as ex:
        for d, kind, res in ex.map(run, jobs):
            results.setdefault((d, kind), []).append(res)
    if os.environ.get("WPT_DETAILS"):
        # Every file's result, for finding what to fix.
        with open(os.environ["WPT_DETAILS"], "w") as f:
            json.dump({f"{d} {kind}": rows for (d, kind), rows in results.items()}, f, indent=1)
    print(f"{'directory':<28} {'kind':<7} {'files':>6} {'passed':>8}")
    grand = {"script": [0, 0], "ref": [0, 0]}
    for (d, kind), rows in sorted(results.items()):
        if kind == "script":
            subs = [s for _, tests, _ in rows if tests for _, s in tests]
            passed = sum(1 for s in subs if s == "PASS")
            total = len(subs) + sum(1 for _, tests, _ in rows if tests is None)
            print(f"{d:<28} {'script':<7} {len(rows):>6} {passed:>5}/{total:<6} subtests ({sum(1 for _, t, _ in rows if t is None)} files gave no result)")
        else:
            passed, total = sum(1 for _, ok, _ in rows if ok), len(rows)
            print(f"{d:<28} {'ref':<7} {len(rows):>6} {passed:>5}/{total:<6} pages match their reference")
        grand[kind][0] += passed
        grand[kind][1] += total
    for k, (p, t) in grand.items():
        if t:
            print(f"TOTAL {k}: {p}/{t} ({p * 100 / t:.1f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
