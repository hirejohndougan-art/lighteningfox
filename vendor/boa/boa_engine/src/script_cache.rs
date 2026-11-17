//! Ironswift change: an in-memory cache of compiled scripts (build-checklist §14, batch M).
//!
//! Every page gets a fresh [`Context`](crate::Context), and every page evaluates the same browser
//! scripts (Ironswift's prelude) and often the same site scripts (a site's bundle on each of its
//! pages, a repeat visit). Parsing and compiling them again each time is most of their cost, so
//! [`Script::parse_cached`](crate::Script::parse_cached) keeps the compiled code of scripts it has
//! seen on this thread and reuses it.
//!
//! Compiling a script against a realm's global scope adds the script's top-level `let`, `const`
//! and `class` bindings to that scope, and the compiled code refers to them by position. So an
//! entry is only reused when the new realm's global scope is exactly what the original realm's
//! was before the original compile; the cache then sets the new scope to what the original was
//! after the compile, which is what compiling again would have produced.
//!
//! Scripts that mention `eval(` are never cached: code compiled at run time inside a cached
//! script would resolve names against the original realm's scopes.

use std::cell::RefCell;
use std::hash::Hasher;

use boa_ast::scope::ScopeSnapshot;
use boa_gc::Gc;
use rustc_hash::{FxHashMap, FxHasher};

use crate::{spanned_source_text::SourceText, vm::CodeBlock};

/// Scripts shorter than this are parsed every time; they are cheap and often one-off
/// (event handler strings, `setTimeout` strings).
pub(crate) const MIN_LEN: usize = 2048;

/// Rough memory budget: compiled code takes about three times its source size.
const BUDGET: usize = 24 << 20;
const COST_PER_BYTE: usize = 3;

pub(crate) struct Hit {
    pub(crate) codeblock: Gc<CodeBlock>,
    pub(crate) source_text: SourceText,
    pub(crate) after: ScopeSnapshot,
}

struct Entry {
    source: Box<[u8]>,
    flags: u32,
    before: ScopeSnapshot,
    after: ScopeSnapshot,
    codeblock: Gc<CodeBlock>,
    source_text: SourceText,
    last_use: u64,
}

#[derive(Default)]
struct Cache {
    map: FxHashMap<u64, Vec<Entry>>,
    cost: usize,
    tick: u64,
    hits: u64,
    misses: u64,
}

impl Drop for Cache {
    fn drop(&mut self) {
        // At thread exit the garbage collector's own thread-local may already be gone; leak the
        // entries rather than touch it.
        std::mem::forget(std::mem::take(&mut self.map));
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

fn key(source: &[u8]) -> u64 {
    let mut h = FxHasher::default();
    h.write(source);
    h.write_usize(source.len());
    h.finish()
}

/// Whether `source` may be cached at all.
pub(crate) fn cacheable(source: &[u8]) -> bool {
    if source.len() < MIN_LEN {
        return false;
    }
    // Any `eval` followed by optional spaces and `(`.
    let mut rest = source;
    while let Some(i) = rest.windows(4).position(|w| w == b"eval") {
        let after = &rest[i + 4..];
        let next = after.iter().find(|c| !c.is_ascii_whitespace());
        if next == Some(&b'(') {
            return false;
        }
        rest = after;
    }
    true
}

/// Finds compiled code for `source` compiled with `flags` against a global scope equal to
/// `before`.
pub(crate) fn lookup(source: &[u8], flags: u32, before: &ScopeSnapshot) -> Option<Hit> {
    CACHE
        .try_with(|c| {
            let mut c = c.borrow_mut();
            c.tick += 1;
            let tick = c.tick;
            let found = c.map.get_mut(&key(source)).and_then(|list| {
                list.iter_mut()
                    .find(|e| e.flags == flags && *e.source == *source && e.before == *before)
                    .map(|e| {
                        e.last_use = tick;
                        Hit {
                            codeblock: e.codeblock.clone(),
                            source_text: e.source_text.clone(),
                            after: e.after.clone(),
                        }
                    })
            });
            if found.is_some() {
                c.hits += 1;
            } else {
                c.misses += 1;
            }
            found
        })
        .ok()
        .flatten()
}

/// Remembers freshly compiled code for `source`.
pub(crate) fn insert(
    source: &[u8],
    flags: u32,
    before: ScopeSnapshot,
    after: ScopeSnapshot,
    codeblock: Gc<CodeBlock>,
    source_text: SourceText,
) {
    let _ = CACHE.try_with(|c| {
        let mut c = c.borrow_mut();
        let cost = source.len() * COST_PER_BYTE;
        if cost > BUDGET / 4 {
            return;
        }
        c.tick += 1;
        let entry = Entry {
            source: source.into(),
            flags,
            before,
            after,
            codeblock,
            source_text,
            last_use: c.tick,
        };
        c.map.entry(key(source)).or_default().push(entry);
        c.cost += cost;
        // Evict the least recently used entries until under budget.
        while c.cost > BUDGET {
            let Some((k, i, last)) = c
                .map
                .iter()
                .flat_map(|(k, list)| list.iter().enumerate().map(move |(i, e)| (*k, i, e.last_use)))
                .min_by_key(|&(_, _, last)| last)
            else {
                break;
            };
            let _ = last;
            let list = c.map.get_mut(&k).expect("key just found");
            let e = list.swap_remove(i);
            if list.is_empty() {
                c.map.remove(&k);
            }
            c.cost -= e.source.len() * COST_PER_BYTE;
        }
    });
}

/// `(hits, misses, entries)` so far on this thread.
#[must_use]
pub fn stats() -> (u64, u64, usize) {
    CACHE
        .try_with(|c| {
            let c = c.borrow();
            (c.hits, c.misses, c.map.values().map(Vec::len).sum())
        })
        .unwrap_or_default()
}

/// Drops every cached script on this thread (for memory pressure).
pub fn clear() {
    let _ = CACHE.try_with(|c| {
        let mut c = c.borrow_mut();
        c.map.clear();
        c.cost = 0;
    });
}
