//! Shaping a page's words across CPU cores before layout.
//!
//! Turning words into positioned glyphs (HarfBuzz-style shaping, `Fonts::shape`) is the
//! biggest single cost of laying out text-heavy pages, and each word is shaped on its own.
//! Layout itself walks the page in order on the window's thread, so before it starts, this
//! pass finds the words it will measure, decides on the window's thread which font face each
//! character uses (`Fonts::plan_shape`, which may load fallback fonts), shapes them all on the
//! style pool (src/parallel.rs), and puts the results in the shaping cache that layout and
//! painting read. A word guessed differently here than layout splits it is simply shaped later
//! by layout, as before, so the pass can only save time, never change what's drawn.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use rayon::prelude::*;

use crate::dom::{Dom, NodeData};
use crate::font::Fonts;
use crate::style::{Display, Style};

/// Fewer new words than this: shape them as layout goes (starting the threads costs more).
const MIN_WORDS: usize = 64;
/// At most this many words per pass (the shaping cache holds 30 000).
const MAX_WORDS: usize = 20_000;

thread_local! {
    /// The styles (address, count) and node count of the last pass: layout runs again for
    /// every resize and animation frame, and the words only change with a restyle.
    static LAST: Cell<(usize, usize, usize)> = const { Cell::new((0, 0, 0)) };
}

/// Shape the words of `dom` that aren't shaped yet, in parallel.
pub fn run(dom: &Dom, styles: &[Rc<Style>], fonts: &Fonts) {
    let Some(pool) = crate::parallel::pool() else { return };
    let stamp = (styles.as_ptr() as usize, styles.len(), dom.nodes.len());
    if LAST.with(|l| l.replace(stamp)) == stamp || !crate::parallel::worth_it(dom.nodes.len()) {
        return;
    }
    let plans = plan(dom, styles, fonts);
    if plans.len() < MIN_WORDS {
        return;
    }
    let shared: Vec<crate::font::PlanRef> = plans.iter().map(|p| p.share()).collect();
    let shaped: Vec<crate::font::Shaped> = pool.install(|| shared.par_iter().with_min_len(16).map(|p| p.run()).collect());
    drop(shared);
    for (p, s) in plans.into_iter().zip(shaped) {
        fonts.insert_shaped(p.key, p.text, s);
    }
}

/// The words layout will measure (split the way `layout::Ctx::flatten` splits them) that
/// aren't shaped yet.
fn plan(dom: &Dom, styles: &[Rc<Style>], fonts: &Fonts) -> Vec<crate::font::ShapePlan> {
    let mut plans = Vec::new();
    // Words already looked at in this style (most words on a page repeat).
    let mut raw: HashSet<(u8, u32, u8, &str)> = HashSet::new();
    let mut seen: HashSet<(u8, u32, String)> = HashSet::new();
    let mut want = |font: u8, size: f32, text: String, plans: &mut Vec<crate::font::ShapePlan>| {
        if plans.len() >= MAX_WORDS || text.is_empty() {
            return;
        }
        let key = (font, size.to_bits(), text);
        if seen.contains(&key) {
            return;
        }
        let plan = fonts.plan_shape(font, &key.2, size);
        seen.insert(key);
        plans.extend(plan);
    };
    let mut stack: Vec<usize> = dom.nodes[0].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let Some(st) = styles.get(id) else { continue };
        match &dom.nodes[id].data {
            NodeData::Element { .. } => {
                if st.display != Display::None {
                    stack.extend(dom.nodes[id].children.iter().rev().copied());
                }
            }
            NodeData::Text(t) => {
                // Letter-spaced text is measured one letter at a time.
                if st.text.letter_spacing != 0.0 {
                    continue;
                }
                let font = fonts.resolve(st.family.as_deref(), st.bold, st.italic, st.mono);
                let size = st.font_size;
                if st.pre {
                    for line in t.replace('\t', "    ").split('\n') {
                        want(font, size, crate::layout::transform(line, st.text_transform), &mut plans);
                    }
                    continue;
                }
                for word in t.split_ascii_whitespace() {
                    if !raw.insert((font, size.to_bits(), st.text_transform, word)) {
                        continue;
                    }
                    let word = crate::layout::transform(word, st.text_transform);
                    for p in crate::text::pieces(&word, &st.text, None) {
                        want(font, size, p.text, &mut plans);
                    }
                }
            }
            _ => {}
        }
        if plans.len() >= MAX_WORDS {
            break;
        }
    }
    plans
}

#[cfg(test)]
mod tests {
    /// Words shaped ahead in parallel are exactly what shaping them in order gives.
    #[test]
    fn same_shapes_as_in_order() {
        let fonts = crate::font::Fonts::load().expect("fonts");
        let words = ["Hello", "world,", "kerning:", "AVATAR", "Wave", "ﬁne", "naïve", "café", "x²", "emoji😀", "日本語", "Привет", "123.45", "ǅ", "e\u{301}"];
        let plans: Vec<_> = words.iter().filter_map(|w| fonts.plan_shape(crate::font::REGULAR, w, 16.0)).collect();
        assert!(plans.len() >= 10, "{}", plans.len());
        let shared: Vec<_> = plans.iter().map(|p| p.share()).collect();
        let ahead: Vec<_> = std::thread::scope(|s| shared.iter().map(|p| s.spawn(|| p.run())).collect::<Vec<_>>().into_iter().map(|h| h.join().unwrap()).collect());
        let fresh = crate::font::Fonts::load().expect("fonts");
        for (p, a) in plans.iter().zip(&ahead) {
            let b = fresh.shape(crate::font::REGULAR, &p.text, 16.0);
            assert_eq!(format!("{a:?}"), format!("{:?}", *b), "{}", p.text);
        }
    }
}
