//! Styling across CPU cores.
//!
//! Selector matching (which rules apply to which element) is most of the time it takes to
//! style a big page, and it doesn't depend on any other element's computed style. So on big
//! pages `style::compute` first matches every element at once on a small pool of threads
//! (Rayon), and then cascades in tree order on the window's thread as before (inheritance
//! needs the parent's style first). Styles stay `Rc` and never leave the window's thread:
//! workers see only the DOM, the stylesheets and a copy of the :hover/:active/:focus/:visited
//! state (`style::MatchState`).
//!
//! Layout stays on one thread: it is a few percent of the work on the same pages (text
//! shaping and font caches are per thread), so it isn't worth splitting yet.
//!
//! `IRONSWIFT_STYLE_THREADS=n` sets the number of threads (1 = match on the window's thread
//! only); `IRONSWIFT_STYLE_PARALLEL_MIN=n` the smallest page (in nodes) styled in parallel.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use crate::dom::{Dom, NodeId};
use crate::style::{MatchCx, MatchState, Matches};

/// Pages with fewer nodes than this are styled on one thread: starting the workers costs
/// more than it saves.
const MIN_NODES: usize = 1500;
/// At most this many threads (a 4 GB laptop has 2–4 cores; big machines gain little more).
const MAX_THREADS: usize = 8;

/// Set by tests (0 = not set: the default, or IRONSWIFT_STYLE_PARALLEL_MIN).
static MIN: AtomicUsize = AtomicUsize::new(0);

fn min_nodes() -> usize {
    static FROM_ENV: OnceLock<usize> = OnceLock::new();
    match MIN.load(Ordering::Relaxed) {
        0 => *FROM_ENV.get_or_init(|| std::env::var("IRONSWIFT_STYLE_PARALLEL_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(MIN_NODES)),
        n => n,
    }
}

pub fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = std::env::var("IRONSWIFT_STYLE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(cores.min(MAX_THREADS));
        if threads <= 1 {
            return None;
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("style-{i}"))
            // Matching :has() and long descendant chains recurses; give it the room the
            // window's thread has.
            .stack_size(8 << 20)
            .build()
            .ok()
    })
    .as_ref()
}

/// Is a page with `nodes` nodes worth styling in parallel?
pub fn worth_it(nodes: usize) -> bool {
    nodes >= min_nodes() && pool().is_some()
}

/// For tests: style pages of any size in parallel (1), or never (usize::MAX); 0 = back to
/// the default.
#[cfg(test)]
pub fn set_min_nodes(n: usize) -> usize {
    MIN.swap(n, Ordering::Relaxed)
}

/// Match the rules for each element in `todo` on the pool. The result is indexed by node id;
/// None for elements not in `todo` and for those that must be matched in order (@container).
pub fn match_all<'a>(cx: &MatchCx<'a>, dom: &Dom, todo: &[NodeId], blooms: &[crate::selmatch::Bloom]) -> Vec<Option<Matches<'a>>> {
    let mut out: Vec<Option<Matches<'a>>> = Vec::new();
    out.resize_with(dom.nodes.len(), || None);
    let Some(pool) = pool() else { return out };
    let state = MatchState::capture();
    let found: Vec<(NodeId, Option<Matches<'a>>)> = pool.install(|| {
        todo.par_iter()
            .with_min_len(32)
            .map_init(
                || {
                    state.install();
                    Vec::new()
                },
                |candidates, &id| (id, cx.match_element(dom, id, &blooms[id], None, candidates)),
            )
            .collect()
    });
    for (id, m) in found {
        out[id] = m;
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    /// Styles from parallel matching are exactly the ones from matching in order.
    #[test]
    fn same_styles_as_one_thread() {
        let mut html = String::from("<!doctype html><style>");
        for i in 0..120 {
            let _ = write!(
                html,
                ".c{i} > p:nth-child({n}) span {{ color: rgb({i}, 0, 0) }} \
                 ul li:not(.z{i}) a[href^='http'] {{ padding-left: {p}px }} \
                 section:has(.e{e}) .d{d} {{ margin: {m}px }} \
                 .c{i} p::before {{ content: 'x{i}' }} \
                 @media (min-width: 600px) {{ .k{k} em {{ font-weight: bold }} }} \
                 @media (max-width: 300px) {{ .k{k} em {{ font-style: italic }} }}",
                n = i % 5 + 1,
                p = i % 7,
                e = i % 11,
                d = i % 13,
                m = i % 3,
                k = i % 17
            );
        }
        html.push_str("</style><body>");
        for s in 0..60 {
            let _ = write!(html, "<section class='c{s} k{}'><div class='d{}'>", s % 17, s % 13);
            for j in 0..8 {
                let _ = write!(html, "<p class='e{}'><span>t{j}</span> <em>e</em> <a href='http://x/{j}'>l</a></p>", j % 11);
            }
            html.push_str("<ul>");
            for j in 0..5 {
                let _ = write!(html, "<li class='z{j}'><a href='https://y/{j}'>i</a></li>");
            }
            html.push_str("</ul></div></section>");
        }
        let (body, css) = html.split_once("</style>").unwrap();
        let sheets = vec![crate::css::parse(css.trim_start_matches("<!doctype html><style>"))];
        let _ = body;
        let mut dom = crate::html::parse(&html);
        crate::css::set_viewport(800.0, 600.0);
        let dump = |styles: &[std::rc::Rc<crate::style::Style>]| styles.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>();
        let saved = super::set_min_nodes(usize::MAX);
        crate::restyle::clear();
        let one = dump(&crate::style::compute(&dom, &sheets));
        super::set_min_nodes(1);
        crate::restyle::clear();
        let many = dump(&crate::style::compute(&dom, &sheets));
        assert!(dom.nodes.len() > 1500);
        let same = |one: &[String], many: &[String], label: &str| {
            assert_eq!(one.len(), many.len());
            for (i, (a, b)) in one.iter().zip(many).enumerate() {
                assert_eq!(a, b, "node {i} ({label})");
            }
        };
        same(&one, &many, "first pass");
        // Restyling after a change reuses unchanged elements and matches the rest in parallel.
        let sec = dom.find_all("section")[3];
        dom.set_attr(sec, "class", "c7 k2");
        let many = dump(&crate::style::compute(&dom, &sheets));
        super::set_min_nodes(usize::MAX);
        crate::restyle::clear();
        let one = dump(&crate::style::compute(&dom, &sheets));
        super::set_min_nodes(saved);
        same(&one, &many, "after a class change");
    }
}
