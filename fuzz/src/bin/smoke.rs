//! A quick mutation fuzzer that runs on stable Rust (no libFuzzer, no coverage guidance).
//! It starts from the repository's test pages, mutates them at random, and feeds every
//! result to each parser. A panic or a very slow input is saved under
//! `artifacts/smoke/<target>-<n>` so `cargo fuzz run <target> <file>` can replay it.
//!
//!   cargo run --release --bin smoke -- [iterations] [seed]

use std::panic;
use std::path::Path;
use std::time::{Duration, Instant};

type Target = (&'static str, fn(&[u8]));

const TARGETS: &[Target] = &[
    ("html", ironswift_fuzz::html_document),
    ("html_fragment", ironswift_fuzz::html_fragment),
    ("xhtml", ironswift_fuzz::xhtml),
    ("css", ironswift_fuzz::css),
    ("javascript", ironswift_fuzz::javascript),
];

/// Pieces of syntax spliced in by the mutator: the constructs parsers get wrong.
const DICT: &[&str] = &[
    "<", ">", "</", "/>", "<!--", "-->", "<!DOCTYPE html>", "<![CDATA[", "]]>", "<?xml ?>", "&", "&amp;", "&#x", "&#", ";", "\"", "'", "=",
    "<table>", "<tr>", "<td>", "</table>", "<p>", "</p>", "<b>", "</b>", "<i>", "<a href=x>", "</a>", "<form>", "<select>", "<option>",
    "<template>", "</template>", "<template shadowrootmode=open>", "<slot>", "<svg>", "<math>", "<foreignObject>", "<mi>", "<annotation-xml encoding=text/html>",
    "<script>", "</script>", "<style>", "</style>", "<textarea>", "<title>", "<plaintext>", "<noscript>", "<iframe>", "<frameset>", "<html>", "<body>", "<head>",
    "{", "}", "(", ")", "[", "]", ":", "::", ",", "*", "@media", "@supports", "@import", "@font-face", "@layer", "@keyframes", "!important",
    ":has(", ":not(", ":is(", ":nth-child(", "calc(", "var(--", "url(", "/*", "*/", "\\", "\u{0}", "\u{feff}", "\u{2028}",
    "function", "=>", "class", "async", "await", "yield", "`${", "/re/g", "new.target", "import(", "export default", "?.", "??=", "#priv",
];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

fn mutate(rng: &mut Rng, input: &[u8], corpus: &[Vec<u8>]) -> Vec<u8> {
    let mut v = input.to_vec();
    for _ in 0..1 + rng.below(6) {
        let at = rng.below(v.len() + 1);
        match rng.below(8) {
            0 if !v.is_empty() => {
                let i = rng.below(v.len());
                v[i] ^= 1 << rng.below(8);
            }
            1 if !v.is_empty() => {
                let end = (at + 1 + rng.below(16)).min(v.len());
                if at < end {
                    v.drain(at..end);
                }
            }
            2 => v.insert(at, rng.next() as u8),
            3 | 4 | 5 => {
                let d = DICT[rng.below(DICT.len())].as_bytes();
                v.splice(at..at, d.iter().copied());
            }
            6 if !v.is_empty() => {
                // Repeat a chunk (deep nesting, long runs).
                let start = rng.below(v.len());
                let end = (start + 1 + rng.below(32)).min(v.len());
                let chunk = v[start..end].to_vec();
                for _ in 0..1 + rng.below(40) {
                    v.splice(at..at, chunk.iter().copied());
                }
            }
            _ => {
                // Splice in part of another seed.
                let other = &corpus[rng.below(corpus.len())];
                if !other.is_empty() {
                    let s = rng.below(other.len());
                    let e = (s + 1 + rng.below(200)).min(other.len());
                    v.splice(at..at, other[s..e].iter().copied());
                }
            }
        }
    }
    v.truncate(64 * 1024);
    v
}

fn seeds(root: &Path) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = DICT.iter().map(|d| d.as_bytes().to_vec()).collect();
    out.push(b"<p>Hello <b>world</i> &amp; <table><td>cell</table>".to_vec());
    out.push(b"a > b:hover, .x ~ #y::before { color: red !important; margin: calc(1px + 2%) }".to_vec());
    out.push(b"for (let i = 0; i < 3; i++) { console.log(`x${i}`); }".to_vec());
    let mut stack = vec![root.join("tests")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with("reference") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "html" || x == "css" || x == "xhtml") {
                if let Ok(b) = std::fs::read(&p) {
                    if b.len() < 256 * 1024 {
                        out.push(b);
                    }
                }
            }
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iterations: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(20_000);
    let seed: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0x1205_5_1f7);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let corpus = seeds(&root);
    let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/smoke");
    println!("{} seeds, {iterations} iterations per target, seed {seed}", corpus.len());
    panic::set_hook(Box::new(|_| {}));
    let mut rng = Rng(seed | 1);
    let mut failures = 0;
    for &(name, f) in TARGETS {
        let started = Instant::now();
        let (mut panics, mut slow, mut slowest) = (0, 0, Duration::ZERO);
        for i in 0..iterations {
            let base = &corpus[rng.below(corpus.len())];
            let input = mutate(&mut rng, base, &corpus);
            // A stack overflow aborts the process before anything can be saved, so the input
            // about to run is written out first.
            let _ = std::fs::create_dir_all(&artifacts);
            let _ = std::fs::write(artifacts.join(format!("last-{name}")), &input);
            let t = Instant::now();
            let r = panic::catch_unwind(|| f(&input));
            let took = t.elapsed();
            slowest = slowest.max(took);
            let bad = if r.is_err() {
                panics += 1;
                Some("panic")
            } else if took > Duration::from_secs(2) {
                slow += 1;
                Some("slow")
            } else {
                None
            };
            if let Some(kind) = bad {
                let _ = std::fs::create_dir_all(&artifacts);
                let file = artifacts.join(format!("{name}-{kind}-{i}"));
                let _ = std::fs::write(&file, &input);
                if panics + slow <= 5 {
                    println!("  {name}: {kind} on input saved as {}", file.display());
                }
            }
        }
        failures += panics + slow;
        println!(
            "{name:<14} {iterations} inputs in {:.1}s: {panics} panics, {slow} over 2s (slowest {} ms)",
            started.elapsed().as_secs_f32(),
            slowest.as_millis()
        );
    }
    if failures > 0 {
        std::process::exit(1);
    }
}
