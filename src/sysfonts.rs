//! Fonts installed on the computer, found by name for CSS `font-family`.
//!
//! The list comes from fontconfig (`fc-list`), the same database every Linux desktop uses,
//! read once and kept in the profile folder (`font-index.tsv`) so the sandboxed frame
//! process, which can't run programs, can use it too. Common Windows and Mac font names
//! (Arial, Helvetica, Times New Roman…) map to their metric-compatible free versions, and the
//! generic families (`serif`, `sans-serif`, `monospace`) to fontconfig's choices.
//!
//! With "Resist fingerprinting" on, pages only get a fixed list of widely installed fonts,
//! so a site can't tell computers apart by which fonts they have.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub path: String,
    pub index: u32,
    /// CSS weight (100…900); a variable font reports the middle of its range.
    pub weight: u16,
    pub italic: bool,
}

#[derive(Default, Debug)]
pub struct Index {
    /// Lower-case family name -> its faces.
    pub families: HashMap<String, Vec<Face>>,
    /// Generic families -> the installed family fontconfig picks for them.
    pub generic: HashMap<String, String>,
}

/// Families every page may use when fingerprinting protection is on (they're on almost every
/// Linux system, or are aliases resolved to them).
const COMMON: &[&str] = &[
    "dejavu sans", "dejavu serif", "dejavu sans mono", "liberation sans", "liberation serif", "liberation mono",
    "noto sans", "noto serif", "noto sans mono", "noto color emoji", "freesans", "freeserif", "freemono",
];

/// Well-known names and the free fonts with the same measurements.
const ALIASES: &[(&str, &[&str])] = &[
    ("arial", &["liberation sans", "arimo"]),
    ("helvetica", &["liberation sans", "arimo"]),
    ("helvetica neue", &["liberation sans", "arimo"]),
    ("times new roman", &["liberation serif", "tinos"]),
    ("times", &["liberation serif", "tinos"]),
    ("courier new", &["liberation mono", "cousine"]),
    ("courier", &["liberation mono", "cousine"]),
    ("calibri", &["carlito"]),
    ("cambria", &["caladea"]),
];

/// fontconfig's weight scale -> CSS weight.
fn css_weight(fc: f32) -> u16 {
    const MAP: &[(f32, u16)] = &[(0.0, 100), (40.0, 200), (50.0, 300), (75.0, 350), (80.0, 400), (100.0, 500), (180.0, 600), (200.0, 700), (205.0, 800), (210.0, 900)];
    MAP.iter().min_by(|a, b| (a.0 - fc).abs().total_cmp(&(b.0 - fc).abs())).map_or(400, |m| m.1)
}

/// Parse `fc-list` output in the format `parse_line` expects.
fn parse(text: &str) -> Index {
    let mut idx = Index::default();
    for line in text.lines() {
        if let Some(g) = line.strip_prefix("#generic\t") {
            if let Some((k, v)) = g.split_once('\t') {
                idx.generic.insert(k.to_string(), v.to_lowercase());
            }
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 6 || f[4].is_empty() {
            continue;
        }
        // Variable fonts give a range: "[100 900]".
        let num = |s: &str| -> f32 {
            let s = s.trim().trim_start_matches('[').trim_end_matches(']');
            let parts: Vec<f32> = s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
            if parts.len() == 2 { (parts[0] + parts[1]) / 2.0 } else { parts.first().copied().unwrap_or(80.0) }
        };
        let face = Face { path: f[4].to_string(), index: f[5].trim().parse().unwrap_or(0), weight: css_weight(num(f[2])), italic: num(f[3]) >= 100.0 };
        for name in f[0].split(',') {
            let name = name.trim().to_lowercase();
            if !name.is_empty() {
                let list = idx.families.entry(name).or_default();
                if !list.contains(&face) {
                    list.push(face.clone());
                }
            }
        }
    }
    idx
}

fn index_file() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("font-index.tsv"))
}

/// Ask fontconfig for the installed fonts (and its picks for the generic families).
fn from_fontconfig() -> Option<String> {
    let out = std::process::Command::new("fc-list").arg("--format=%{family}\t%{style}\t%{weight}\t%{slant}\t%{file}\t%{index}\n").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    for g in ["serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui", "emoji"] {
        if let Ok(o) = std::process::Command::new("fc-match").args(["--format=%{family[0]}", g]).output() {
            let fam = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !fam.is_empty() {
                text.push_str(&format!("#generic\t{g}\t{fam}\n"));
            }
        }
    }
    Some(text)
}

/// The installed fonts (looked up once per run).
pub fn index() -> &'static Index {
    static IDX: OnceLock<Index> = OnceLock::new();
    IDX.get_or_init(|| {
        if std::env::var("IRONSWIFT_SYSTEM_FONTS").is_ok_and(|v| v == "0") {
            return Index::default();
        }
        let text = match from_fontconfig() {
            Some(t) => {
                // Kept for the frame process, which can't start fc-list itself.
                if let Some(p) = index_file() {
                    let tmp = p.with_extension("tmp");
                    if std::fs::write(&tmp, &t).is_ok() {
                        let _ = std::fs::rename(&tmp, &p);
                    }
                }
                t
            }
            None => index_file().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default(),
        };
        parse(&text)
    })
}

/// The installed family to use for a CSS family name (lower-case), if any. `strict`: only the
/// common families (fingerprinting protection).
pub fn family(idx: &Index, name: &str, strict: bool) -> Option<String> {
    let allowed = |f: &str| !strict || COMMON.contains(&f);
    let name = name.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_lowercase();
    if idx.families.contains_key(&name) && allowed(&name) {
        return Some(name);
    }
    if let Some((_, targets)) = ALIASES.iter().find(|(a, _)| *a == name) {
        return targets.iter().find(|t| idx.families.contains_key(**t)).map(|t| t.to_string());
    }
    let generic = match name.as_str() {
        "ui-serif" => "serif",
        "ui-sans-serif" | "-apple-system" | "blinkmacsystemfont" => "sans-serif",
        "ui-monospace" => "monospace",
        g => g,
    };
    // Serif goes with the built-in DejaVu Sans when it's there (the same look everywhere).
    if generic == "serif" && idx.families.contains_key("dejavu serif") {
        return Some("dejavu serif".into());
    }
    let fam = idx.generic.get(generic)?;
    (idx.families.contains_key(fam) && allowed(fam)).then(|| fam.clone())
}

/// The face of a family closest to the wanted weight and style.
pub fn best_face<'a>(faces: &'a [Face], bold: bool, italic: bool) -> Option<&'a Face> {
    let want: i32 = if bold { 700 } else { 400 };
    faces.iter().min_by_key(|f| ((f.italic != italic) as i32) * 10_000 + (f.weight as i32 - want).abs() + if bold && (f.weight as i32) < 600 { 500 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Liberation Sans\tRegular\t80\t0\t/f/LiberationSans-Regular.ttf\t0\n\
        Liberation Sans\tBold\t200\t0\t/f/LiberationSans-Bold.ttf\t0\n\
        Liberation Sans\tItalic\t80\t100\t/f/LiberationSans-Italic.ttf\t0\n\
        Inter,Inter Black\tBlack\t210\t0\t/f/Inter-Black.otf\t0\n\
        Inter\tRegular\t80\t0\t/f/Inter-Regular.otf\t0\n\
        Roboto Flex\tRegular\t[0 210]\t0\t/f/RobotoFlex.ttf\t0\n\
        Noto Sans CJK SC,Noto Sans CJK\tRegular\t80\t0\t/f/NotoSansCJK.ttc\t2\n\
        #generic\tserif\tDejaVu Serif\n\
        #generic\tsans-serif\tLiberation Sans\n";

    #[test]
    fn index_and_lookup() {
        let idx = parse(SAMPLE);
        assert_eq!(idx.families["liberation sans"].len(), 3);
        assert_eq!(idx.families["noto sans cjk sc"][0].index, 2);
        assert_eq!(idx.families["inter black"][0].weight, 900);
        assert_eq!(idx.families["roboto flex"][0].weight, 500);
        assert_eq!(family(&idx, "\"Inter\"", false).as_deref(), Some("inter"));
        assert_eq!(family(&idx, "Inter", true), None, "not a common font: hidden when resisting fingerprinting");
        assert_eq!(family(&idx, "Arial", true).as_deref(), Some("liberation sans"));
        assert_eq!(family(&idx, "sans-serif", false).as_deref(), Some("liberation sans"));
        assert_eq!(family(&idx, "serif", false), None, "fontconfig's pick isn't installed in this sample");
        assert_eq!(family(&idx, "Comic Sans MS", false), None);
        let f = &idx.families["liberation sans"];
        assert!(best_face(f, true, false).unwrap().path.ends_with("Bold.ttf"));
        assert!(best_face(f, false, true).unwrap().path.ends_with("Italic.ttf"));
        assert!(best_face(f, false, false).unwrap().path.ends_with("Regular.ttf"));
    }
}
