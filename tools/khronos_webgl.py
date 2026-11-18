#!/usr/bin/env python3
"""Run the Khronos WebGL conformance tests against Ironswift.

    git clone --depth 1 --filter=blob:none --sparse https://github.com/KhronosGroup/WebGL khronos
    (cd khronos && git sparse-checkout set sdk/tests/js sdk/tests/resources \\
        sdk/tests/conformance sdk/tests/conformance2)
    python3 tools/khronos_webgl.py khronos [conformance2 | conformance | conformance2/textures …]

Follows the suite's 00_test_list.txt files (WebGL 2.0.1 tests: --min-version and
--max-version are honoured), serves sdk/tests on localhost and runs each page with
`--png`, media and animations playing (IRONSWIFT_WAIT_FOR) until the page calls
finishTest(). The suite's own harness hooks (window.parent.webglTestHarness) are filled in by a
reporter put in front of js/js-test-pre.js in the checkout. A page passes when every check in
it passed; the count of checks is reported too.

Test profiles turn off anti-fingerprinting (it nudges readPixels values by 1).
KHR_TIMEOUT=60 gives each page 60 seconds instead of 40; KHR_DETAILS=out.json writes every
page's result; KHR_JOBS=n runs n pages at a time (default: the number of cores).
"""
import concurrent.futures, functools, http.server, json, os, re, subprocess, sys, tempfile, threading

HERE = os.path.dirname(os.path.abspath(__file__))
BROWSER = os.environ.get("IRONSWIFT_BIN") or os.path.join(HERE, "..", "target", "release", "ironswift")
VERSION = (2, 0, 1)
MARK = "/* ironswift khronos reporter */"
REPORTER = MARK + r"""
window.webglTestHarness = {
  passed: 0, failed: 0, failures: [],
  reportResults: function (path, success, msg, skipped) {
    if (skipped) return;
    if (success) this.passed++; else { this.failed++; if (this.failures.length < 5) this.failures.push(String(msg).slice(0, 160)); }
  },
  notifyFinished: function () {
    console.log("KHR-RESULT " + JSON.stringify({ passed: this.passed, failed: this.failed, failures: this.failures }));
    window.__khrDone = true;
  },
};
"""


def version(s):
    return tuple(int(x) for x in s.split("."))


def tests_in(root, rel):
    """Pages listed by 00_test_list.txt under rel (a list file's folder, or a single page)."""
    if rel.endswith(".html"):
        return [rel]
    path = os.path.join(root, rel, "00_test_list.txt")
    out = []
    for line in open(path):
        line = line.split("//")[0].split("#")[0].strip()
        if not line:
            continue
        words = line.split()
        name, ok = words[-1], True
        for i, w in enumerate(words[:-1]):
            if w == "--min-version" and version(words[i + 1]) > VERSION:
                ok = False
            if w == "--max-version" and version(words[i + 1]) < VERSION:
                ok = False
        if not ok:
            continue
        sub = os.path.join(rel, name) if rel else name
        if name.endswith(".txt"):
            out += tests_in(root, os.path.dirname(sub))
        elif os.path.exists(os.path.join(root, sub)):
            out.append(sub)
    return out


def install_reporter(root):
    pre = os.path.join(root, "js", "js-test-pre.js")
    text = open(pre).read()
    if MARK not in text:
        open(pre, "w").write(REPORTER + text)


def serve(root):
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=root))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv.server_address[1]


def run(base, rel, tmp):
    timeout = int(os.environ.get("KHR_TIMEOUT", "40"))
    prof = tempfile.mkdtemp(dir=tmp)
    # Canvas and WebGL readouts carry anti-fingerprinting noise by default (a value in a
    # hundred moved by 1); the suite measures WebGL itself, so it's off in test profiles.
    with open(os.path.join(prof, "settings.tsv"), "w") as f:
        f.write("resist_fingerprinting\t0\n")
    env = dict(os.environ, IRONSWIFT_PROFILE=prof, NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost",
               IRONSWIFT_WAIT_FOR="window.__khrDone === true", IRONSWIFT_WAIT_SECS=str(max(1, timeout - 5)),
               IRONSWIFT_WAIT_EXPIRED="window.webglTestHarness && window.webglTestHarness.notifyFinished()")
    try:
        p = subprocess.run([BROWSER, "--width", "800", "--png", os.path.join(prof, "x.png"), f"{base}/{rel}"],
                           capture_output=True, text=True, timeout=timeout, env=env)
        log = p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return rel, None, "timeout"
    m = re.search(r"KHR-RESULT (\{.*\})", log)
    if not m:
        return rel, None, "no result"
    return rel, json.loads(m.group(1)), None


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    root = os.path.join(os.path.abspath(sys.argv[1]), "sdk", "tests")
    dirs = sys.argv[2:] or ["conformance2"]
    install_reporter(root)
    port = serve(root)
    base = f"http://127.0.0.1:{port}"
    tmp = tempfile.mkdtemp()
    details = {}
    jobs = int(os.environ.get("KHR_JOBS", os.cpu_count() or 2))
    # A folder's subfolders are counted separately (conformance2 → conformance2/attribs, …).
    expanded = []
    for d in dirs:
        listed = os.path.join(root, d, "00_test_list.txt")
        subs = [w.split()[-1] for w in open(listed).read().splitlines() if w.strip().endswith("00_test_list.txt")] if os.path.exists(listed) else []
        expanded += [os.path.join(d, os.path.dirname(x)) for x in subs] or [d]
        if subs:
            loose = [r for r in tests_in(root, d) if os.path.dirname(r) == d]
            expanded += loose
    dirs = expanded
    totals = [0, 0, 0, 0]
    for d in dirs:
        pages = tests_in(root, d)
        passed = checks_ok = checks = 0
        noresult = []
        with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
            for rel, r, err in ex.map(lambda r: run(base, r, tmp), pages):
                details[rel] = r if r else {"error": err}
                if r is None:
                    noresult.append(rel)
                    continue
                checks += r["passed"] + r["failed"]
                checks_ok += r["passed"]
                if r["failed"] == 0 and r["passed"] > 0:
                    passed += 1
        print(f"{d:40} {passed}/{len(pages)} pages pass, {checks_ok}/{checks} checks ({len(noresult)} pages gave no result)", flush=True)
        for i, v in enumerate((passed, len(pages), checks_ok, checks)):
            totals[i] += v
        if os.environ.get("KHR_DETAILS"):
            json.dump(details, open(os.environ["KHR_DETAILS"], "w"), indent=1)
    print(f"{'TOTAL':40} {totals[0]}/{totals[1]} pages pass, {totals[2]}/{totals[3]} checks", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
