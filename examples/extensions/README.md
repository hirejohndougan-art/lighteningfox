# Example extensions

Two small extensions to try Ironswift's WebExtensions support:

- `darkmode/`: a content-script stylesheet and script, `chrome.storage`, and messages
  between the content script and its background (`runtime.sendMessage`, `tabs.sendMessage`).
- `blocker/`: a background that blocks requests with "ads" in their path and redirects
  `old.png` to `new.png` with a blocking `webRequest.onBeforeRequest` listener.

Load them by copying (or linking) them into `<profile>/extensions/`, or for one run:

    python3 -m http.server 8765 -d examples/extensions/site &
    IRONSWIFT_EXTENSIONS=$PWD/examples/extensions/darkmode:$PWD/examples/extensions/blocker \
      ./target/release/ironswift --png out.png http://127.0.0.1:8765/
