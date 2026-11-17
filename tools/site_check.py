#!/usr/bin/env python3
"""Live-site check and site-compatibility tracker.

    python3 tools/site_check.py                  # every site in tools/sites.json
    python3 tools/site_check.py https://a.com …  # or your own list
    python3 tools/site_check.py --tracker        # also update docs/SITE_COMPAT.md

Run it on a computer with normal internet access, after `cargo build --release`. For each site
it saves Ironswift's rendering, the time it took, how many script errors happened and whether it
crashed. If Chrome or Chromium is installed it also saves Chrome's rendering, compares the two
(a 0-100 likeness score) and shows them side by side. Open site-check/report.html when it's done.

With --tracker it grades each site and rewrites docs/SITE_COMPAT.md from docs/site-compat.json:
  Works        renders, no script errors, looks like Chrome (likeness 85+)
  Usable       renders and mostly looks right (likeness 60+), or has a few script errors
  Broken       renders, but looks badly wrong
  Crashes      Ironswift crashed or hung
  Not loaded   the site couldn't be reached (network, blocking, site error)
  Rendered     rendered, but there was no Chrome to compare with
A grade or note written by hand in docs/site-compat.json ("grade_override", "notes") is kept.
The weekly workflow .github/workflows/site-compat.yml runs this on GitHub's machines.

Ironswift uses a fresh, temporary profile for this, so your own cookies and history aren't touched.
"""
import datetime, html, json, os, re, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..")
BROWSER = os.environ.get("IRONSWIFT_BIN") or os.path.join(ROOT, "target", "release", "ironswift")
OUT = os.path.join(os.getcwd(), "site-check")
TRACKER_JSON = os.path.join(ROOT, "docs", "site-compat.json")
TRACKER_MD = os.path.join(ROOT, "docs", "SITE_COMPAT.md")
WIDTH = 1280
COMPARE_H = 2000  # compare the top of each page (what you see first, and a bit more)


def load_sites():
    with open(os.path.join(HERE, "sites.json")) as f:
        return json.load(f)


def slug(url):
    return re.sub(r"[^a-z0-9]+", "-", url.lower().split("://", 1)[-1]).strip("-")[:60]


def chrome():
    for name in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        path = shutil.which(name)
        if path:
            return path
    return None


def run_ironswift(url, png, profile):
    env = dict(os.environ, IRONSWIFT_PROFILE=profile, IRONSWIFT_NO_UPDATES="1")
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
        "crashed": "panicked" in out or "[crash]" in out or code == "timeout" or (isinstance(code, int) and code < 0),
        "unreachable": "[net] couldn't load" in out,
    }


def run_chrome(exe, url, png):
    try:
        subprocess.run([exe, "--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars", f"--window-size={WIDTH},{COMPARE_H}",
                        f"--screenshot={png}", url], capture_output=True, timeout=90)
    except subprocess.TimeoutExpired:
        pass
    return os.path.exists(png)


def likeness(a_png, b_png):
    """How alike two screenshots look, 0-100: both are cut to the top COMPARE_H pixels,
    shrunk, and compared pixel by pixel (exact layout doesn't need to match to score well)."""
    try:
        from PIL import Image, ImageChops, ImageStat
    except ImportError:
        return None
    try:
        a, b = Image.open(a_png).convert("L"), Image.open(b_png).convert("L")
    except OSError:
        return None
    h = min(COMPARE_H, max(a.height, b.height))

    def norm(im):
        canvas = Image.new("L", (WIDTH, h), 255)
        canvas.paste(im.crop((0, 0, min(WIDTH, im.width), min(h, im.height))), (0, 0))
        return canvas.resize((WIDTH // 16, max(1, h // 16)), Image.BILINEAR)

    diff = ImageChops.difference(norm(a), norm(b))
    return round(100 - ImageStat.Stat(diff).mean[0] / 255 * 100 * 2.5, 1)  # x2.5: small diffs matter


def grade(mine, score, have_chrome):
    if mine["crashed"]:
        return "Crashes"
    if mine["unreachable"]:
        return "Not loaded"
    if not mine["ok"]:
        return "Crashes" if mine["code"] != 0 else "Not loaded"
    if not have_chrome or score is None:
        return "Rendered"
    if score >= 85 and mine["errors"] == 0:
        return "Works"
    if score >= 60:
        return "Usable"
    return "Broken"


def version():
    with open(os.path.join(ROOT, "Cargo.toml")) as f:
        m = re.search(r'^version\s*=\s*"([^"]+)"', f.read(), re.M)
    try:
        rev = subprocess.run(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    except OSError:
        rev = ""
    return (m.group(1) if m else "?") + (f" ({rev})" if rev else "")


def update_tracker(sites, results):
    try:
        with open(TRACKER_JSON) as f:
            data = json.load(f)
    except (OSError, ValueError):
        data = {"sites": {}}
    today = datetime.date.today().isoformat()
    ver = version()
    by_url = data.setdefault("sites", {})
    for site in sites:
        url = site["url"]
        entry = by_url.setdefault(url, {})
        entry["name"], entry["category"] = site.get("name", url), site.get("category", "Other")
        r = results.get(url)
        if not r:
            continue
        if entry.get("grade") and entry.get("grade") != r["grade"]:
            entry["previous_grade"] = entry["grade"]
        entry.update({"grade": r["grade"], "likeness": r["likeness"], "script_errors": r["errors"], "seconds": round(r["time"], 1), "checked": today, "version": ver})
    data["last_run"] = today
    with open(TRACKER_JSON, "w") as f:
        json.dump(data, f, indent=1, sort_keys=True)
        f.write("\n")
    write_tracker_md(sites, data)


ORDER = ["Works", "Usable", "Rendered", "Broken", "Not loaded", "Crashes", "Not checked"]


def write_tracker_md(sites, data):
    by_url = data.get("sites", {})
    counts = {g: 0 for g in ORDER}
    rows_by_cat = {}
    for site in sites:
        e = by_url.get(site["url"], {})
        g = e.get("grade_override") or e.get("grade") or "Not checked"
        counts[g] = counts.get(g, 0) + 1
        change = f" (was {e['previous_grade']})" if e.get("previous_grade") and not e.get("grade_override") else ""
        like = f"{e['likeness']:.0f}" if isinstance(e.get("likeness"), (int, float)) else "–"
        errs = str(e.get("script_errors", "–"))
        rows_by_cat.setdefault(site.get("category", "Other"), []).append(
            f"| [{site.get('name', site['url'])}]({site['url']}) | {g}{change} | {like} | {errs} | {e.get('checked', '–')} | {e.get('notes', '')} |"
        )
    total = len(sites)
    summary = ", ".join(f"{counts[g]} {g.lower()}" for g in ORDER if counts.get(g))
    out = [
        "# Site compatibility",
        "",
        "How well popular sites work in Ironswift. Generated by `python3 tools/site_check.py --tracker`",
        "(run weekly on GitHub by `.github/workflows/site-compat.yml`); edit the site list in",
        "`tools/sites.json`. To add a note or correct a grade by hand, set `notes` or `grade_override`",
        "for the site in `docs/site-compat.json`; the next run keeps them.",
        "",
        f"**{total} sites:** {summary}. Last run: {data.get('last_run', 'not yet')}.",
        "",
        "Grades: **Works** looks like Chrome and has no script errors; **Usable** mostly right;",
        "**Broken** renders but badly wrong; **Crashes** crashed or hung; **Not loaded** couldn't be",
        "reached; **Rendered** loaded but wasn't compared with Chrome. *Likeness* is how close the",
        "first screenful looks to Chrome's (0-100).",
        "",
    ]
    for cat, rows in rows_by_cat.items():
        out += [f"## {cat}", "", "| Site | Grade | Likeness | Script errors | Checked | Notes |", "|---|---|---|---|---|---|", *rows, ""]
    with open(TRACKER_MD, "w") as f:
        f.write("\n".join(out))


def main():
    args = sys.argv[1:]
    tracker = "--tracker" in args
    args = [a for a in args if a != "--tracker"]
    if "--tracker-only" in args:
        # Rewrite the markdown from the JSON (after editing notes by hand).
        sites = load_sites()
        try:
            data = json.load(open(TRACKER_JSON))
        except (OSError, ValueError):
            data = {"sites": {}}
        write_tracker_md(sites, data)
        print(f"Wrote {TRACKER_MD}")
        return 0
    if not os.path.exists(BROWSER):
        print("Build Ironswift first: cargo build --release")
        return 2
    all_sites = load_sites()
    sites = [{"url": u, "name": u, "category": "Other"} for u in args] if args else all_sites
    os.makedirs(OUT, exist_ok=True)
    exe = chrome()
    print(f"Checking {len(sites)} sites" + (f", with {os.path.basename(exe)} for comparison" if exe else " (no Chrome found: Ironswift only)"))
    rows, results = [], {}
    with tempfile.TemporaryDirectory() as profile:
        for site in sites:
            url = site["url"]
            s = slug(url)
            mine_png, theirs_png = os.path.join(OUT, f"{s}-ironswift.png"), os.path.join(OUT, f"{s}-chrome.png")
            for p in (mine_png, theirs_png):
                if os.path.exists(p):
                    os.remove(p)
            mine = run_ironswift(url, mine_png, profile)
            theirs = run_chrome(exe, url, theirs_png) if exe else False
            score = likeness(mine_png, theirs_png) if mine["ok"] and theirs and not mine["unreachable"] else None
            g = grade(mine, score, bool(theirs))
            mine.update({"likeness": score, "grade": g})
            results[url] = mine
            print(f"  {g:<10} {mine['time']:5.1f}s  {mine['errors']:>3} script errors  likeness {score if score is not None else '–':>5}  {url}")
            rows.append((site, s, mine, theirs))

    cells = []
    for site, s, mine, theirs in rows:
        url = site["url"]
        errs = "".join(f"<li><code>{html.escape(e[:200])}</code></li>" for e in mine["first_errors"])
        like = f" · likeness {mine['likeness']}" if mine["likeness"] is not None else ""
        cells.append(f"""
<section>
  <h2><a href="{html.escape(url)}">{html.escape(site.get('name', url))}</a> <span class="g g-{mine['grade'].split()[0].lower()}">{mine['grade']}</span></h2>
  <p class="meta">{html.escape(url)} · {mine['time']:.1f} s · {mine['scripts']} scripts, {mine['errors']} script errors{like}</p>
  {f'<details><summary>First script errors</summary><ul>{errs}</ul></details>' if errs else ''}
  <div class="pair">
    <figure><figcaption>Ironswift</figcaption>{f'<img src="{s}-ironswift.png">' if mine['ok'] else '<p>No rendering</p>'}</figure>
    {f'<figure><figcaption>Chrome</figcaption><img src="{s}-chrome.png"></figure>' if theirs else ''}
  </div>
</section>""")
    counts = {}
    for _, _, m, _ in rows:
        counts[m["grade"]] = counts.get(m["grade"], 0) + 1
    summary = ", ".join(f"{n} {g.lower()}" for g, n in sorted(counts.items(), key=lambda kv: ORDER.index(kv[0]) if kv[0] in ORDER else 99))
    page = f"""<!DOCTYPE html><html><head><meta charset="utf-8"><title>Ironswift site check</title><style>
body{{font-family:sans-serif;margin:24px;color:#1f2937}} section{{border-top:1px solid #ddd;padding:12px 0}}
h2{{font-size:17px;margin:0 0 4px}} .meta{{color:#555;margin:0 0 8px}} .pair{{display:flex;gap:12px;align-items:flex-start}}
figure{{margin:0;flex:1;min-width:0}} figcaption{{font-weight:bold;margin-bottom:4px}}
img{{width:100%;max-height:900px;object-fit:cover;object-position:top;border:1px solid #ccc}}
.g{{font-size:12px;padding:2px 8px;border-radius:10px;background:#e5e7eb;vertical-align:middle}}
.g-works{{background:#dcfce7}} .g-usable{{background:#fef9c3}} .g-broken,.g-crashes{{background:#fee2e2}}
</style></head><body><h1>Ironswift site check</h1>
<p>{len(rows)} sites: {summary}. Compare each pair: layout, missing pieces, broken text.</p>
{''.join(cells)}</body></html>"""
    with open(os.path.join(OUT, "report.html"), "w") as f:
        f.write(page)
    print(f"\n{summary}. Report: {os.path.join(OUT, 'report.html')}")
    if tracker:
        update_tracker(all_sites, results)  # only sites from tools/sites.json go in the tracker
        print(f"Tracker updated: {TRACKER_MD}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
