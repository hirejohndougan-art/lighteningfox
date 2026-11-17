# Ironswift browser UI — English (the source language; every other file follows its ids).

## Right-click menu
menu-open-link-new-tab = Open link in new tab
menu-copy-link = Copy link address
menu-open-image-new-tab = Open image in new tab
menu-copy-image = Copy image address
menu-copy = Copy
menu-back = Back
menu-forward = Forward
menu-reload = Reload
menu-select-all = Select all
menu-find = Find in page…
menu-save-pdf = Save as PDF…
menu-print = Print…
menu-view-source = View page source
menu-inspect = Inspect

## Downloads panel
dl-progress = { $pct }% · { $got } of { $total }
dl-so-far = { $got } so far
dl-done = Done · { $size } · click to open
dl-failed = Failed: { $error }
dl-cancelled = Cancelled
dl-show-all = Show all downloads
dl-open-folder = Open Downloads folder

## Short notes under the toolbar
toast-zoom = Zoom { $pct }%  ·  Ctrl+0 resets
toast-printed = { $n ->
    [one] Sent 1 page to the printer
   *[other] Sent { $n } pages to the printer
}
toast-pdf-saved = Saved { $file } ({ $n ->
    [one] 1 page
   *[other] { $n } pages
}) to Downloads
toast-pdf-no-printer = No printer answered; saved { $file } ({ $n ->
    [one] 1 page
   *[other] { $n } pages
}) to Downloads
toast-pdf-failed = Couldn't save the PDF: { $error }

## Window and tabs
window-loading = Loading { $target } — Ironswift
private-badge = Private

## Find in page
find-none = No matches
find-count = { $current } of { $total }
find-hint = Enter: next  ·  Shift+Enter: previous  ·  Esc: close

## Error pages
err-try-again = Try again
err-settings = Settings
err-details = Details
err-dns-title = This site can't be found
err-dns-detail = No address was found for { $host }.
err-dns-tip1 = Check the spelling of the address.
err-dns-tip2 = Check that you're connected to the internet.
err-dns-tip3 = The site may no longer exist.
err-timeout-title = The site took too long to answer
err-timeout-detail = { $host } didn't respond in time.
err-timeout-tip1 = Try again in a moment; the site may be busy.
err-net-tip = Check your internet connection.
err-cert-title = This connection isn't secure
err-cert-detail = { $host } sent a security certificate Ironswift can't trust, so the page wasn't loaded. Someone could be trying to read or change what you send.
err-cert-tip1 = If this is your own or your company's site, its certificate may have expired or not be installed.
err-cert-tip2 = Don't enter passwords or payment details on this site.
err-refused-title = The site refused to connect
err-refused-detail = { $host } isn't accepting connections.
err-refused-tip1 = The site may be down or not running on this port.
err-refused-tip2 = Try again later.
err-proxy-title = Couldn't connect through the proxy
err-proxy-detail = The proxy server didn't let the request through.
err-proxy-tip = Check the proxy in Settings (Network).
err-type-title = Ironswift can't show this file
err-type-detail = This kind of file can't be shown in a tab.
err-other-title = Couldn't load this page
err-other-detail = Something went wrong loading { $host }.
err-other-tip = Try again with ↻.

## Settings (language and accessibility)
set-language = Language
set-language-auto = Same as the desktop
set-language-hint = For Ironswift's menus and messages, and what sites are told you read. Takes full effect the next time Ironswift starts.
set-high-contrast = High contrast
set-high-contrast-hint = Draws pages and the browser in black, white and yellow with no background images, for easier reading.

## Screen readers (window, tabs, toolbar) and the settings page
a11y-tabs = Tabs
a11y-toolbar = Navigation
a11y-address = Address and search bar
tab-untitled = New tab
button-back = Back
button-forward = Forward
button-reload = Reload
settings-title = Settings
settings-sub = Saved on this computer only.
settings-saved = Settings saved.
settings-cache-cleared = Cached files cleared ({ $mb } MB freed).
settings-startup = Startup and search
settings-home = Home page
settings-search-engine = Search engine
settings-restore-tabs = Reopen the tabs from last time
settings-bookmarks-bar = Show the bookmarks bar
settings-memory = Memory
settings-sleep = Put background tabs to sleep after (minutes, 0 = never)
settings-sleep-hint = A sleeping tab frees its page and reloads when you come back to it.
settings-gpu = Draw with the graphics card
settings-gpu-hint = Smoother scrolling and less work for the processor; the graphics driver uses about 40–70 MB more memory. Takes effect the next time Ironswift starts.
settings-privacy = Privacy
settings-tpc = Share cookies with embedded sites
settings-tpc-hint = Off by default: a site embedded in other sites (an ad or tracking network, a video player) gets a separate cookie jar on each site, so it can't follow you around.
settings-strip = Remove tracking tags from links
settings-strip-hint = utm_source, fbclid, gclid and similar.
settings-gpc = Send Global Privacy Control
settings-gpc-hint = Asks sites not to sell or share your data.
settings-clear-exit = Delete cookies and site data when Ironswift closes
settings-clear-exit-hint = Signs you out of sites each time; history and bookmarks are kept.
settings-cookies-allow = Always allow cookies from these sites
settings-cookies-allow-hint = Even when they're embedded in other sites (for sign-in or comment boxes that need them).
settings-cookies-block = Never allow cookies from these sites
settings-cookies-session = Delete these sites' cookies when Ironswift closes
settings-partition-hint = Other sites embedded in a page get their own separate cookie jar for that page, so they can't follow you from site to site.
settings-cookie-sites = Sites with cookies ({ $count })
settings-clear-data = Clear browsing data now
settings-clear-data-hint = History, cookies, site data and cached files. Bookmarks and settings stay.
settings-private-hint = For browsing that keeps nothing at all, open a private window with Ctrl+Shift+N.
settings-blocking = Blocking and safety
settings-block-ads = Block ads
settings-block-ads-hint = EasyList: ad requests are stopped and empty ad boxes hidden. Pages load faster.
settings-block-trackers = Block trackers
settings-block-trackers-hint = EasyPrivacy: analytics, tracking pixels and fingerprinting scripts.
settings-no-blocking = Turn blocking off on these sites
settings-safe = Warn me before dangerous sites
settings-safe-hint = Sites on the URLhaus malware list and the Phishing Army list. The check happens on this computer; nothing is sent anywhere.
settings-rfp = Resist fingerprinting
settings-rfp-hint = Sites see a common screen size, the standard fonts and slightly varied canvas readouts, so they can't recognize your computer by its details.
settings-update-lists = Update lists now
settings-permissions-hint = Camera, microphone, location and notifications: sites ask first.
settings-updates = Updates
settings-check-updates = Check for new versions once a day
settings-check-now = Check now
settings-appearance = Appearance
settings-min-font = Minimum font size (px, 0 = none)
settings-min-font-hint = Makes small print on pages larger. Ctrl + and Ctrl − zoom a whole site.
settings-reduce-motion = Reduce motion
settings-reduce-motion-hint = Asks sites to keep animations to a minimum.
settings-default-browser = Default browser
settings-make-default = Make Ironswift my default browser
settings-network = Network
settings-proxy = Proxy
settings-proxy-system = System settings
settings-proxy-none = No proxy
settings-proxy-manual = Use this proxy
settings-proxy-address = Proxy address (for "Use this proxy")
settings-proxy-hint = "System settings" follows your desktop's proxy and the http_proxy / https_proxy / no_proxy variables. Secure sites are checked against your computer's certificate store.
settings-clear-cache = Clear cached files
settings-cache-size = ({ $mb } MB saved so pages load faster)
settings-save = Save settings
settings-history = History
settings-bookmarks = Bookmarks
settings-cookies-deleted = Cookies from { $site } deleted.
settings-cookies-allowed = { $site } may always use cookies.
settings-cookies-blocked = { $site } can't use cookies any more (its cookies were deleted).
settings-cookies-session-set = { $site }'s cookies will be deleted when Ironswift closes.
settings-cookies-default = { $site } uses the normal cookie rules.
settings-permissions-reset = Permissions for { $site } reset.
