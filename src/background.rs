//! Background layers: `background: url(a.png) top left no-repeat, linear-gradient(…), #fff`
//! and the comma lists of `background-image`, `-size`, `-position` and `-repeat`.
//! The first layer is drawn on top; the color goes under all of them.

use crate::css;
use crate::style::{Background, BgImage, BgSize, Color, Len, Style};

/// One layer as painted.
pub struct Layer<'a> {
    pub image: &'a BgImage,
    pub size: BgSize,
    pub pos: (Len, Len),
    pub repeat: (bool, bool),
}

impl Background {
    /// Every image layer, top first. Sizes, positions and repeats come from their lists,
    /// which repeat when shorter than the image list (as CSS says).
    pub fn layers(&self) -> Vec<Layer<'_>> {
        let Some(first) = &self.image else { return Vec::new() };
        let pick = |i: usize| Layer {
            image: if i == 0 { first } else { &self.more[i - 1] },
            size: if self.sizes.is_empty() { self.size } else { self.sizes[i % self.sizes.len()] },
            pos: if self.positions.is_empty() { self.pos } else { self.positions[i % self.positions.len()] },
            repeat: if self.repeats.is_empty() { self.repeat } else { self.repeats[i % self.repeats.len()] },
        };
        (0..=self.more.len()).map(pick).collect()
    }

    /// Every image URL in the layers (for loading).
    pub fn urls(&self) -> impl Iterator<Item = &String> {
        self.image.iter().chain(self.more.iter()).filter_map(|i| match i {
            BgImage::Url(u) => Some(u),
            _ => None,
        })
    }
}

/// `background-image: a, b, none`: the first becomes `image`, the rest `more`.
pub fn set_images(bg: &mut Background, v: &str) {
    let imgs: Vec<Option<BgImage>> = css::split_top(v, ',').iter().map(|l| crate::style::parse_bg_image(l.trim())).collect();
    bg.image = imgs.first().cloned().flatten();
    // A `none` layer in the middle is dropped (nothing to draw).
    bg.more = imgs.into_iter().skip(1).flatten().collect();
}

fn list<T>(v: &str, mut f: impl FnMut(&str) -> Option<T>) -> Vec<T> {
    css::split_top(v, ',').iter().filter_map(|p| f(p.trim())).collect()
}

/// `background-size`, `-position`, `-repeat` with one or more comma-separated values.
pub fn set_sizes(bg: &mut Background, v: &str, fs: f32) {
    let l = list(v, |p| Some(crate::style::parse_bg_size(p, fs)));
    if let Some(f) = l.first() {
        bg.size = *f;
    }
    bg.sizes = if l.len() > 1 { l } else { Vec::new() };
}

pub fn set_positions(bg: &mut Background, v: &str, fs: f32) {
    let l = list(v, |p| Some(crate::style::parse_bg_pos(p, fs)));
    if let Some(f) = l.first() {
        bg.pos = *f;
    }
    bg.positions = if l.len() > 1 { l } else { Vec::new() };
}

pub fn set_repeats(bg: &mut Background, v: &str) {
    let l = list(v, crate::style::parse_repeat);
    if let Some(f) = l.first() {
        bg.repeat = *f;
    }
    bg.repeats = if l.len() > 1 { l } else { Vec::new() };
}

/// The `background` shorthand: every layer's image, position / size and repeat; the color
/// from the last layer.
pub fn apply_shorthand(st: &mut Style, v: &str, fs: f32) {
    st.bg = Background::default();
    st.background = Color::TRANSPARENT;
    if v.trim().eq_ignore_ascii_case("none") {
        return;
    }
    let (mut images, mut sizes, mut positions, mut repeats) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for layer in css::split_top(v, ',') {
        let (pos_part, size_part) = match css::split_top(&layer, '/').as_slice() {
            [a, b, ..] => (a.clone(), Some(b.clone())),
            [a] => (a.clone(), None),
            _ => (String::new(), None),
        };
        let (mut image, mut repeat, mut pos_words, mut size_words) = (None, (true, true), Vec::new(), Vec::new());
        let mut word = |t: &str, in_size: bool, size_words: &mut Vec<String>| {
            let tl = t.to_ascii_lowercase();
            if t.is_empty() {
            } else if let Some(i) = crate::style::parse_bg_image(t) {
                image = Some(i);
            } else if let Some(c) = crate::style::parse_color(&tl) {
                st.background = c;
                st.bg.color = c;
            } else if let Some(r) = crate::style::parse_repeat(&tl) {
                repeat = r;
            } else if ["scroll", "fixed", "local", "border-box", "padding-box", "content-box", "text"].contains(&tl.as_str()) {
            } else if in_size {
                size_words.push(tl);
            } else {
                pos_words.push(tl);
            }
        };
        for t in css::split_top(&pos_part, ' ') {
            word(t.trim(), false, &mut size_words);
        }
        if let Some(size) = &size_part {
            // The size part may be followed by repeat/color words.
            let words = css::split_top(size, ' ');
            for (k, t) in words.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).enumerate() {
                // After two size words, the rest is something else (repeat, image…).
                word(t, k < 2, &mut size_words);
            }
        }
        images.push(image);
        sizes.push(if size_words.is_empty() { BgSize::Auto } else { crate::style::parse_bg_size(&size_words.join(" "), fs) });
        positions.push(if pos_words.is_empty() { (Len::Pct(0.0), Len::Pct(0.0)) } else { crate::style::parse_bg_pos(&pos_words.join(" "), fs) });
        repeats.push(repeat);
    }
    // Keep only layers with an image, with their own size/position/repeat.
    let keep: Vec<usize> = (0..images.len()).filter(|&i| images[i].is_some()).collect();
    let Some(&first) = keep.first() else { return };
    st.bg.image = images[first].clone();
    st.bg.size = sizes[first];
    st.bg.pos = positions[first];
    st.bg.repeat = repeats[first];
    if keep.len() > 1 {
        st.bg.more = keep[1..].iter().map(|&i| images[i].clone().unwrap()).collect();
        st.bg.sizes = keep.iter().map(|&i| sizes[i]).collect();
        st.bg.positions = keep.iter().map(|&i| positions[i]).collect();
        st.bg.repeats = keep.iter().map(|&i| repeats[i]).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_with_several_layers() {
        let mut st = Style::default();
        apply_shorthand(&mut st, "url(a.png) right top / 20px no-repeat, linear-gradient(red, blue), #123456", 16.0);
        assert_eq!(st.bg.color, Color::rgb(0x12, 0x34, 0x56));
        let layers = st.bg.layers();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].image, &BgImage::Url("a.png".into()));
        assert_eq!(layers[0].repeat, (false, false));
        assert_eq!(layers[0].pos, (Len::Pct(100.0), Len::Pct(0.0)));
        assert_eq!(layers[0].size, BgSize::Size(Len::Px(20.0), Len::Auto));
        assert!(matches!(layers[1].image, BgImage::Gradient(_)));
        assert_eq!(layers[1].repeat, (true, true));
        apply_shorthand(&mut st, "repeating-conic-gradient(#ddd 0 25%, #fff 0 50%) 0 0 / 20px 20px", 16.0);
        assert_eq!(st.bg.layers()[0].size, BgSize::Size(Len::Px(20.0), Len::Px(20.0)));
    }

    #[test]
    fn longhand_lists_repeat() {
        let mut bg = Background::default();
        set_images(&mut bg, "url(a.png), url(b.png), url(c.png)");
        set_repeats(&mut bg, "no-repeat, repeat-x");
        let l = bg.layers();
        assert_eq!(l.len(), 3);
        assert_eq!((l[0].repeat, l[1].repeat, l[2].repeat), ((false, false), (true, false), (false, false)));
        assert_eq!(bg.urls().count(), 3);
    }
}

