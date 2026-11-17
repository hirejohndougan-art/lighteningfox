# Debugging a live site offline

The chats' workspaces can't reach most websites, so sites are **recorded** on GitHub's machines
and **replayed** here. A recording holds every response Ironswift got, so the site behaves the
same way every time, with no network.

## Recordings that already exist

Branch `x1-data2` holds AP News, MDN, Bing, BBC News, New York Times, The Guardian,
rust-lang.org and WordPress.org, recorded 2026-11-17 with Ironswift at commit c1ce921 (before
stylesheets added by scripts were applied, so record your site again before relying on it). Each
site has a folder:

    <site>/rec/        the responses (one .meta + .body per request)
    <site>/run.log     Ironswift's output, script errors included
    <site>/iron.png    Ironswift's rendering
    <site>/chrome.png  Chrome's rendering (1280x2000), to compare with
    <site>/bt.txt      backtraces of every thread, only if it hung

Get them (keep them outside the repo):

    git clone -q --depth 1 -b x1-data2 https://github.com/hirejohndougan-art/lighteningfox /tmp/sites

## Replay

    IRONSWIFT_PROFILE=$(mktemp -d) IRONSWIFT_NO_UPDATES=1 IRONSWIFT_REPLAY=/tmp/sites/www-bbc-com-news/rec \
      ./target/release/ironswift --width 1280 --png out.png https://www.bbc.com/news

Use the exact URL that was recorded (`head -1 rec/*.meta` lists them). A request that isn't in the
recording fails with "not in the recording", as if that server were down. That's normal for ads
and trackers, but if a page's own code or CSS is missing (it was never requested because an
earlier error stopped the scripts), record the site again after fixing that error.

Compare with Chrome side by side:

    python3 - <<'E'
    from PIL import Image
    a = Image.open("out.png").convert("RGB").crop((0, 0, 1280, 1100))
    b = Image.open("/tmp/sites/www-bbc-com-news/chrome.png").convert("RGB").crop((0, 0, 1280, 1100))
    c = Image.new("RGB", (2560, 1100), "white"); c.paste(a, (0, 0)); c.paste(b, (1280, 0))
    c.resize((1280, 550)).save("cmp.png")
    E

## Record again

After fixes, or for another site, run the "Record sites" workflow. Chats can't use
`gh workflow run` or `gh run list` (they go through GitHub's GraphQL API, which is blocked
here); use the REST API through `gh api`:

    gh api -X POST repos/hirejohndougan-art/lighteningfox/actions/workflows/record-sites.yml/dispatches \
      -f ref=main -f "inputs[urls]=https://www.bbc.com/news" -f "inputs[branch]=rec-bbc" -f "inputs[ref]=main"
    # ~12 minutes; check on it with:
    gh api "repos/hirejohndougan-art/lighteningfox/actions/workflows/record-sites.yml/runs?per_page=5" \
      --jq '.workflow_runs[] | "\(.id) \(.status) \(.conclusion) \(.created_at)"'
    # the run's log (script errors, hangs): gh api repos/hirejohndougan-art/lighteningfox/actions/runs/<id>/jobs --jq '.jobs[0].id'
    #   then: gh api repos/hirejohndougan-art/lighteningfox/actions/jobs/<job id>/logs
    git clone -q --depth 1 -b rec-bbc https://github.com/hirejohndougan-art/lighteningfox /tmp/rec-bbc

Slow sites keep making requests long after load (NYT's React app takes minutes on Boa to
hydrate and fetch its images): add `-f "inputs[wait]=240"` to keep each page running that many
seconds before the snapshot, so those requests are recorded too. Replay the same way with
`IRONSWIFT_WAIT_FOR=false IRONSWIFT_WAIT_SECS=240`.

`ref` is what gets built: `main`, or a branch you pushed with fixes that aren't on `main` yet.
Name the branch after your task (`rec-x1a`…) so chats don't overwrite each other's. Chats can't
delete branches (the git proxy refuses it): when your task is done, list your recording branch in
your report so John can delete it.

## Finding the cause

- **Script errors** are in the log with where they were thrown (`file.js:line:column`).
  Minified code is on line 1 or 2: print the text around that column from the recorded body
  (`grep -l "<file name>" rec/*.meta` finds it) to see what was called.
- **"not a callable function" / "X is not defined"** usually means a missing web API. Add it
  in a new JS file listed in `EXTRA_APIS` in `src/js.rs` (see `traversal.js`, `cssom.js`,
  `docprops.js`).
- **A hang**: `gdb -p <pid> -batch -ex "thread apply all bt 40"` a few times while it's stuck
  (gdb is installed); the recording's `bt.txt` shows the same from GitHub.
- **Layout or style** problems: cut the page down to a small test page that shows the same
  difference, fix that, and keep it in `tests/sites/` with a test (see the X1 tests at the end of
  `src/jsapi_tests.rs`). No site-specific hacks.
- Check WPT for layout changes: `python3 tools/wpt.py <wpt checkout> css/css-flexbox css/CSS2/floats`
  should stay the same or improve.
