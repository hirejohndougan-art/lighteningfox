# Security policy

Ironswift is an early, experimental browser. **Don't use it as your everyday browser yet**:
it doesn't isolate sites from each other in separate processes, and it doesn't enforce the
same-origin policy for storage or Content Security Policy. Those are on the build checklist
before any release meant for daily use.

## Reporting a vulnerability

Please report security problems privately, not in a public issue:

1. Open the repository on GitHub and go to **Security → Report a vulnerability**
   (GitHub's private vulnerability reporting).
2. Describe what goes wrong, which version or commit you used, and how to reproduce it
   (a small HTML file is ideal).

You'll get a reply within 7 days. Once a fix is ready, it's released and the report is made
public with credit to you, unless you'd rather stay anonymous.

## Supported versions

Only the latest commit on `main` gets security fixes while Ironswift is at version 0.x.

## What counts

Anything that lets a web page do something it shouldn't, for example:

- read another site's cookies, storage or page contents
- run code outside the page, or read or write files on your computer
- crash the browser in a way that could be exploitable (memory corruption, not just a
  panic; Ironswift is written in safe Rust, so these are most likely in dependencies)
- get around blocked third-party cookies, tracking-parameter stripping or pop-up blocking

Crashes that only stop the browser (a Rust panic) are ordinary bugs: open an issue, and
attach the report from about:crashes.
