# Privacy and telemetry

**Ironswift sends no telemetry.** It has no usage statistics, crash uploads, analytics, or
"phone home" checks of any kind, and there is no setting that turns any on. This is a
standing policy: if that ever changes, it will be opt-in only, off by default, and
announced in the changelog before it ships.

## What stays on your computer

Everything Ironswift keeps is in your profile folder (`~/.local/share/ironswift`, or the
folder in `IRONSWIFT_PROFILE`), readable only by you:

- history, bookmarks, settings, open tabs (to restore them)
- cookies and site storage (`localStorage`)
- cached pages and files, and per-site zoom levels
- crash reports and the debug log (shown on about:crashes and about:log; never sent anywhere)

Settings → **Clear browsing data** removes them; a **private window** (Ctrl+Shift+N) keeps
none of it.

## What sites see

Ironswift blocks third-party cookies, strips common tracking parameters (`utm_*`,
`fbclid`, `gclid`…) from links, and sends Global Privacy Control (`Sec-GPC: 1`) with every
request. Sites still see your IP address and what any browser sends (the user agent,
language, screen size).
