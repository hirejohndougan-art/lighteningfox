#!/usr/bin/env python3
"""Live-site check: render real websites with Swiftiron and write a side-by-side report.

    python3 tools/site_check.py                 # the built-in list of sites
    python3 tools/site_check.py https://a.com …  # or your own

Run it on a computer with normal internet access, after `cargo build --release`. For each site it
saves Swiftiron's rendering, the time it took, how many script errors happened and whether it
crashed. If Chrome or Chromium is installed, it also saves Chrome's rendering of the same page, so
the report shows them side by side. Open site-check/report.html when it's done.

Swiftiron uses a fresh, temporary profile for this, so your own cookies and history aren't touched.
"""
import html, os, re, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
BROWSER = os.path.join(HERE, "..", "target", "release", "swiftiron")
OUT = os.path.join(os.getcwd(), "site-check")
WIDTH = 1280

SITES = [
    "https://example.com/",
    "https://en.wikipedia.org/wiki/Web_browser",
    "https://www.wikipedia.org/",
    "https://wordpress.org/",
    "https://accounts.google.com/",          # the Gmail sign-in page
    "https://duckduckgo.com/",
    "https://github.com/",
    "https://old.reddit.com/",
    "https://news.ycombinator.com/",
    "https://text.npr.org/",
    "https://lite.cnn.com/",
    "https://www.bbc.com/news",
    "https://www.nytimes.com/",
    "https://www.weather.gov/",
    "https://developer.mozilla.org/en-US/",
    "https://www.dickinsonhistoricalsociety.org/",
]


def slug(url):
    return re.sub(r"[^a-z0-9]+", "-", url.lower().split("://", 1)[-1]).strip("-")[:60]


def chrome():
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        path = shutil.which(name)
        if path:
            return path
    return None


def run_swiftiron(url, png, profile):
    env = dict(os.environ, SWIFTIRON_PROFILE=profile)
    start = time.time()
    try:
        p = subprocess.run([BROWSER, "--width", str(WIDTH), "--png", png, url], capture_output=True, text=True, timeout=90, env=env)
        out, code = p.stdout + p.stderr, p.returncode
    except subprocess.TimeoutExpired:
        out, code = "", "timeout"
    took = time.time() - start
    m = re.search(r"JS: (\d+) scripts, (\d+) errors", out)
    errors = [l for l in out.splitlines() if l.startswith("[js error]")][:8]
    return {
        "time": took,
        "ok": code == 0 and os.path.exists(png),
        "code": code,
        "scripts": int(m.group(1)) if m else 0,
        "errors": int(m.group(2)) if m else 0,
        "first_errors": errors,
        "crashed": "panicked" in out or "[crash]" in out,
        "unreachable": "[net] couldn't load" in out,
    }


def run_chrome(exe, url, png):
    try:
        subprocess.run([exe, "--headless=new", "--disable-gpu", "--hide-scrollbars", f"--window-size={WIDTH},2000",
                        f"--screenshot={png}", url], capture_output=True, timeout=90)
    except subprocess.TimeoutExpired:
        pass
    return os.path.exists(png)


def main():
    if not os.path.exists(BROWSER):
        print("Build Swiftiron first: cargo build --release")
        return 2
    sites = sys.argv[1:] or SITES
    os.makedirs(OUT, exist_ok=True)
    exe = chrome()
    print(f"Checking {len(sites)} sites" + (f", with {os.path.basename(exe)} for comparison" if exe else " (no Chrome found: Swiftiron only)"))
    rows = []
    with tempfile.TemporaryDirectory() as profile:
        for url in sites:
            s = slug(url)
            mine = run_swiftiron(url, os.path.join(OUT, f"{s}-swiftiron.png"), profile)
            theirs = run_chrome(exe, url, os.path.join(OUT, f"{s}-chrome.png")) if exe else False
            status = "CRASH" if mine["crashed"] else "no-load" if mine["unreachable"] else "ok" if mine["ok"] else f"failed ({mine['code']})"
            print(f"  {status:<8} {mine['time']:5.1f}s  {mine['errors']:>3} script errors  {url}")
            rows.append((url, s, mine, theirs))

    cells = []
    for url, s, mine, theirs in rows:
        status = "Crashed" if mine["crashed"] else "Couldn't load (network or site error)" if mine["unreachable"] else "Rendered" if mine["ok"] else f"Failed ({mine['code']})"
        errs = "".join(f"<li><code>{html.escape(e[:200])}</code></li>" for e in mine["first_errors"])
        cells.append(f"""
<section>
  <h2><a href="{html.escape(url)}">{html.escape(url)}</a></h2>
  <p class="meta"><b>{status}</b> in {mine['time']:.1f} s · {mine['scripts']} scripts, {mine['errors']} script errors</p>
  {f'<details><summary>First script errors</summary><ul>{errs}</ul></details>' if errs else ''}
  <div class="pair">
    <figure><figcaption>Swiftiron</figcaption>{f'<img src="{s}-swiftiron.png">' if mine['ok'] else '<p>No rendering</p>'}</figure>
    {f'<figure><figcaption>Chrome</figcaption><img src="{s}-chrome.png"></figure>' if theirs else ''}
  </div>
</section>""")
    ok = sum(1 for r in rows if r[2]["ok"] and not r[2]["crashed"] and not r[2]["unreachable"])
    page = f"""<!DOCTYPE html><html><head><meta charset="utf-8"><title>Swiftiron site check</title><style>
body{{font-family:sans-serif;margin:24px;color:#1f2937}} section{{border-top:1px solid #ddd;padding:12px 0}}
h2{{font-size:17px;margin:0 0 4px}} .meta{{color:#555;margin:0 0 8px}} .pair{{display:flex;gap:12px;align-items:flex-start}}
figure{{margin:0;flex:1;min-width:0}} figcaption{{font-weight:bold;margin-bottom:4px}}
img{{width:100%;max-height:900px;object-fit:cover;object-position:top;border:1px solid #ccc}}
</style></head><body><h1>Swiftiron site check</h1>
<p>{ok} of {len(rows)} sites rendered without crashing. Compare each pair: layout, missing pieces, broken text.</p>
{''.join(cells)}</body></html>"""
    with open(os.path.join(OUT, "report.html"), "w") as f:
        f.write(page)
    print(f"\n{ok} of {len(rows)} rendered. Report: {os.path.join(OUT, 'report.html')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
