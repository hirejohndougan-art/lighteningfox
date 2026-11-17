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
