#!/usr/bin/env python3
"""Blank out API keys in recorded responses that GitHub's push protection refused.

Usage: scrub_secrets.py PUSH_OUTPUT_FILE  (run in the recordings' folder)

GitHub refuses a push holding something that looks like a secret (the New York Times page has
a public Mapbox token, 2026-11). The refusal names each file and line; on those lines, every
token of a known kind keeps its prefix and the rest becomes "x" (same length, so the page's
code still runs). Prints the files it changed.
"""
import re
import sys

PATTERNS = [
    r"[ps]k\.eyJ[A-Za-z0-9_.-]{20,}",            # Mapbox
    r"AIza[0-9A-Za-z_-]{35}",                    # Google API keys
    r"(?:AKIA|ASIA)[0-9A-Z]{16}",                # AWS access key ids
    r"gh[pousr]_[A-Za-z0-9]{30,}",               # GitHub tokens
    r"xox[abprs]-[A-Za-z0-9-]{10,}",             # Slack
    r"(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{10,}",  # Stripe
    r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",  # JWTs
]
TOKEN = re.compile("|".join(PATTERNS))


def blank(m):
    t = m.group(0)
    keep = 4 if not t.startswith("eyJ") else 3
    return t[:keep] + "x" * (len(t) - keep)


def main():
    out = open(sys.argv[1], encoding="utf-8", errors="replace").read()
    spots = {}
    for path, line in re.findall(r"path:\s*(\S+?):(\d+)", out):
        spots.setdefault(path, set()).add(int(line))
    changed = []
    for path, lines in spots.items():
        try:
            data = open(path, "rb").read().decode("utf-8", errors="surrogateescape").split("\n")
        except OSError:
            continue
        hit = False
        for n in lines:
            if 0 < n <= len(data):
                new = TOKEN.sub(blank, data[n - 1])
                hit |= new != data[n - 1]
                data[n - 1] = new
        if hit:
            open(path, "wb").write("\n".join(data).encode("utf-8", errors="surrogateescape"))
            changed.append(path)
    print("\n".join(changed))


if __name__ == "__main__":
    main()
