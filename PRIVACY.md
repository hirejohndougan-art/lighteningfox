# Privacy and telemetry

**Ironswift sends no telemetry.** It has no usage statistics, crash uploads or analytics,
and there is no setting that turns any on. This is a standing policy: if that ever
changes, it will be opt-in only, off by default, and announced in the changelog before it
ships.

Ironswift makes only these requests of its own, none of which carry anything about you or
what you browse (the servers see your IP address and Ironswift's user agent, as with any
download):

- **Filter lists**, every four days: EasyList and EasyPrivacy (easylist.to), the URLhaus
  malware host list (abuse.ch) and the Phishing Army list. Sites are checked against them
  on your computer; the addresses you visit are never sent anywhere. Turn them off under
  Settings → Blocking and safety.
- **Update check**, once a day: GitHub's public "latest release" page for this repository.
  Turn it off under Settings → Updates.

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

Ironswift blocks ads and trackers with the EasyList and EasyPrivacy filter lists, gives
sites embedded in other sites a separate cookie jar and storage for each site they're
embedded in (so they can't follow you around), strips common tracking parameters
(`utm_*`, `fbclid`, `gclid`…) from links, and sends Global Privacy Control (`Sec-GPC: 1`)
with every request. Scripts see the same answers on every computer for screen size,
processor count and installed fonts (only Ironswift's own fonts and the page's web fonts
can be used). Sites still see your IP address and what any browser sends (the user agent,
language).

Camera, microphone, location and notifications are off until you answer a site's prompt;
your answers are kept per site and can be reset on about:settings.
