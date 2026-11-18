# Security policy

Ironswift is an early, experimental browser. **Don't use it as your everyday browser yet**:
top-level pages from different sites still share one process. Iframes from other sites are
drawn in a separate sandboxed process, and the browser itself runs under a system-call
filter, but full site-per-process isolation is still on the build checklist before any
release meant for daily use.

## How pages are kept apart

| Protection | What it does | Where |
|---|---|---|
| Same-origin policy | Cookies, `localStorage`, `fetch`/XHR and `pushState` always act for the page's own origin, decided by the browser, never by what a script passes in. Embedded sites get storage and cookies partitioned per top-level site. | `js.rs` (`PageCtx`), `cookies.rs` |
| Content Security Policy | `Content-Security-Policy` headers and `<meta>`: `script-src` (nonces, hashes, `'strict-dynamic'`), `style-src`, `img-src`, `font-src`, `connect-src`, `frame-src`, `form-action`, `frame-ancestors`, `upgrade-insecure-requests`, `sandbox`; `eval` only with `'unsafe-eval'`; report-only policies log. | `security.rs` |
| Framing | `X-Frame-Options` and `frame-ancestors`; `<iframe sandbox>` (scripts, same-origin, forms, popups). | `security.rs`, `main.rs` |
| Local files and browser pages | Web pages can't load, read, navigate to or frame `file:` URLs, `about:` pages, `view-source:` or `data:` documents. A local HTML file can read only files in its own folder. | `security.rs`, `net.rs` |
| Process sandbox | The browser: `no_new_privs` and a seccomp filter (no ptrace, mount, module loading, BPF, keyrings, namespaces…). Cross-site iframes: their own process that also can't start programs, and with Landlock can't write any file or read outside system folders and the profile. | `sandbox.rs` |
| Content blocking | EasyList/EasyPrivacy network and element-hiding rules; a warning page for hosts on the URLhaus and Phishing Army lists. | `blocklist.rs` |
| JavaScript JIT | Busy functions are compiled to machine code (Cranelift). Code pages are never writable and executable at once, and compiled code is never patched. Each page's code is freed with its JavaScript context. Settings → "JavaScript JIT" turns it off. | `vendor/boa/boa_engine/src/jit/`, `docs/JIT.md` |
| Fuzzing | The HTML, CSS, JavaScript, CSP and filter-list parsers are fuzzed with cargo-fuzz (`fuzz/README.md`). | `fuzz/` |
| Updates | A daily check for a newer release; per-user installs download it, verify its SHA-256 and install it on exit. | `updates.rs` |

## The JavaScript JIT

Since task J4 the baseline JIT is on by default. A JIT is a classic place for browser exploits,
because a wrong assumption in compiled code can turn a value into a pointer. What limits it here:

- **W^X.** Cranelift writes code into read+write pages and then makes them read+execute; pages
  are never read+write+execute and are never written again (checked by the
  `executable_memory_is_never_writable` test). On ARM64 it uses BTI when the CPU has it.
- **Small surface.** The compiler does number arithmetic, locals, branches and loops itself.
  Every other operation (property access, calls, exceptions) calls the interpreter's own code
  for that instruction, so the compiler holds no separate copy of JavaScript's rules about
  objects to get wrong. Generators, async functions, class constructors and functions over
  32 KB of bytecode are never compiled.
- **Type checks, and no object pointers.** Compiled code reads a value's tag (a few small
  helpers in `compiler.rs`) before doing number arithmetic on it, and never dereferences an
  object pointer itself: a value that isn't a number goes to the interpreter for that
  instruction. Tests feed compiled code values of other types to check they take that path.
- **Differential testing.** test262 gives byte-identical results with the JIT off, on and
  forced on for every function, and the `jit_diff` fuzz target runs each program with and
  without the JIT and fails on any difference.
- **Lifetime.** Each JavaScript context owns its machine code and frees it when the page goes
  away, so code can't outlive the objects it refers to.
- **The sandbox still applies.** Site processes keep their system-call filter and Landlock
  rules with the JIT on. The JIT only needs `mmap`/`mprotect`, which the filter allows.
- **Off switch.** Settings → "JavaScript JIT" (applies at the next start), or `BOA_JIT=off` in
  the environment, runs all JavaScript in the interpreter without rebuilding. Builds without
  the `boa_jit` feature have no JIT at all.

Not done yet: Spectre-style hardening of compiled code (index masking, as V8 and SpiderMonkey
added), and the parts of the code still left to the interpreter. Bugs in compiled code that change a
script's result count as security bugs: please report them privately as below.

## Reporting a vulnerability

Please report security problems privately, not in a public issue:

1. Open the repository on GitHub and go to **Security → Report a vulnerability**
   (GitHub's private vulnerability reporting).
2. Describe what goes wrong, which version or commit you used, and how to reproduce it
   (a small HTML file is ideal).

You'll get a reply within 7 days. Once a fix is ready, it's released and the report is made
public with credit to you, unless you'd rather stay anonymous.

## Supported versions

Only the latest release (and the latest commit on `main`) gets security fixes while
Ironswift is at version 0.x. Installed with `packaging/linux/install.sh`, Ironswift updates
itself: it checks GitHub once a day, downloads a newer release, checks it against the
published SHA-256 sum and installs it when you close it (Settings → Updates).

## What counts

Anything that lets a web page do something it shouldn't, for example:

- read another site's cookies, storage or page contents
- run code outside the page, or read or write files on your computer
- crash the browser in a way that could be exploitable (memory corruption, not just a
  panic; Ironswift is written in safe Rust, so these are most likely in dependencies)
- get around blocked third-party cookies, tracking-parameter stripping or pop-up blocking

Crashes that only stop the browser (a Rust panic) are ordinary bugs: open an issue, and
attach the report from about:crashes.
