#!/usr/bin/env python3
"""Compare pages drawn in-process with the same pages drawn in a site process.

    python3 tools/site_process_check.py [pages…]   # default: the screenshot test pages

Serves tests/ over http on localhost (site processes only take http/https pages), takes each
page's first screen (1024x768) both ways — `--png` as usual, and with IRONSWIFT_SITE_PROCESS=1,
where the page runs in its site's sandboxed process (src/site_snapshot.rs) — and reports the
pages whose pictures differ, with the share of pixels that differ. Differing pictures are saved
to site-process-check/. Needs `cargo build --release` first.
"""
import functools, http.server, os, re, subprocess, sys, threading

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..")
BROWSER = os.environ.get("IRONSWIFT_BIN") or os.path.join(ROOT, "target", "release", "ironswift")
OUT = os.path.join(os.getcwd(), "site-process-check")


def screenshot_pages():
    src = open(os.path.join(ROOT, "src", "main.rs")).read()
    pages = re.findall(r'\("tests/([^"]+\.html)", \d+\)', src)
    real = sorted("real-world/" + f for f in os.listdir(os.path.join(ROOT, "tests", "real-world")) if f.endswith(".html"))
    return list(dict.fromkeys(pages + real))


def serve(root):
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=root))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv.server_address[1]


def shot(url, out, site, profile):
    env = dict(os.environ, IRONSWIFT_PROFILE=profile, NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
    env["IRONSWIFT_SITE_PROCESS"] = "1" if site else "0"
    try:
        p = subprocess.run([BROWSER, "--width", "1024", "--png", out, url], capture_output=True, text=True, timeout=90, env=env)
        return p.returncode == 0 and os.path.exists(out), (p.stdout + p.stderr)[-400:]
    except subprocess.TimeoutExpired:
        return False, "timeout"


def main():
    from PIL import Image, ImageChops
    import tempfile
    pages = sys.argv[1:] or screenshot_pages()
    port = serve(os.path.join(ROOT, "tests"))
    os.makedirs(OUT, exist_ok=True)
    tmp = tempfile.mkdtemp()
    for prof in ("p1", "p2"):
        os.makedirs(os.path.join(tmp, prof), exist_ok=True)
    same, differ, failed = [], [], []
    for page in pages:
        url = f"http://127.0.0.1:{port}/{page}"
        name = page.replace("/", "_")
        a, b = os.path.join(tmp, name + ".in.png"), os.path.join(tmp, name + ".site.png")
        ok_a, log_a = shot(url, a, False, os.path.join(tmp, "p1"))
        ok_b, log_b = shot(url, b, True, os.path.join(tmp, "p2"))
        if not (ok_a and ok_b):
            failed.append((page, log_b if ok_a else log_a))
            print(f"FAILED  {page}")
            continue
        ia, ib = Image.open(a).convert("RGB"), Image.open(b).convert("RGB")
        h = min(768, ia.height, ib.height)
        ia, ib = ia.crop((0, 0, 1024, h)), ib.crop((0, 0, 1024, h))
        diff = ImageChops.difference(ia, ib).convert("L").point(lambda v: 255 if v > 8 else 0)
        share = diff.histogram()[255] / (1024 * h)
        if share == 0:
            same.append(page)
            print(f"same    {page}")
        else:
            differ.append((page, share))
            ia.save(os.path.join(OUT, name + ".in.png"))
            ib.save(os.path.join(OUT, name + ".site.png"))
            print(f"DIFFERS {page}: {share:.2%} of pixels")
    print(f"\n{len(same)} same, {len(differ)} differ, {len(failed)} failed (of {len(pages)})")
    for page, why in failed:
        print(f"  failed {page}: {why.strip().splitlines()[-1] if why.strip() else '?'}")
    return 1 if differ or failed else 0


if __name__ == "__main__":
    sys.exit(main())
