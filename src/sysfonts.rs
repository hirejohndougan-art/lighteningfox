//! Fonts installed on the computer, found by name for CSS `font-family`.
//!
//! On Linux the list comes from fontconfig (`fc-list`), the same database every desktop uses;
//! on Windows and macOS Ironswift reads the font folders itself (`scan`). Either way it is
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
    // On every Windows and Mac computer:
    "arial", "times new roman", "courier new", "verdana", "georgia", "trebuchet ms",
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

/// CSS weight -> fontconfig's scale (the inverse of `css_weight`), for `scan`'s lines.
fn fc_weight(css: u16) -> u16 {
    const MAP: &[(u16, u16)] = &[(100, 0), (200, 40), (300, 50), (350, 75), (400, 80), (500, 100), (600, 180), (700, 200), (800, 205), (900, 210)];
    MAP.iter().min_by_key(|m| m.0.abs_diff(css)).map_or(80, |m| m.1)
}

/// The fonts in one file as lines in the `fc-list` format `parse` reads.
fn describe(path: &std::path::Path, data: &[u8], out: &mut String) {
    use swash::{Style, StringId};
    let Some(coll) = swash::FontDataRef::new(data) else { return };
    for i in 0..coll.len() {
        let Some(font) = coll.get(i) else { continue };
        let strings = font.localized_strings();
        let name = |id| strings.find_by_id(id, Some("en")).or_else(|| strings.find_by_id(id, None)).map(|s| s.to_string());
        let mut names: Vec<String> = [StringId::TypographicFamily, StringId::Family].into_iter().filter_map(name).filter(|n| !n.trim().is_empty()).collect();
        names.dedup();
        if names.is_empty() {
            continue;
        }
        let attrs = font.attributes();
        let italic = !matches!(attrs.style(), Style::Normal);
        out.push_str(&format!("{}\t\t{}\t{}\t{}\t{}\n", names.join(","), fc_weight(attrs.weight().0), if italic { 100 } else { 0 }, path.display(), i));
    }
}

/// Without fontconfig (Windows, macOS): read the font folders ourselves, with the system's
/// usual picks for the generic families.
fn scan() -> String {
    let mut out = String::new();
    let mut stack: Vec<(std::path::PathBuf, u8)> = crate::platform::font_dirs().into_iter().map(|d| (d, 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if depth < 3 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                if let Ok(data) = std::fs::read(&path) {
                    describe(&path, &data, &mut out);
                }
            }
        }
    }
    let generic: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("serif", &["Times", "Times New Roman"]), ("sans-serif", &["Helvetica", "Arial"]), ("monospace", &["Menlo", "Courier New"]), ("system-ui", &["SF Pro", "Helvetica Neue", "Helvetica"]), ("emoji", &["Apple Color Emoji"])]
    } else {
        &[("serif", &["Times New Roman"]), ("sans-serif", &["Arial"]), ("monospace", &["Consolas", "Courier New"]), ("system-ui", &["Segoe UI"]), ("emoji", &["Segoe UI Emoji"])]
    };
    let have = out.to_lowercase();
    for (g, picks) in generic {
        if let Some(f) = picks.iter().find(|f| have.contains(&format!("{}\t", f.to_lowercase())) || have.contains(&format!("{},", f.to_lowercase()))) {
            out.push_str(&format!("#generic\t{g}\t{f}\n"));
        }
    }
    out
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
            None => {
                // Kept from an earlier run (the frame process, or a system without fontconfig);
                // without fontconfig the folders are read again once the copy is a week old.
                let kept = index_file().filter(|p| {
                    cfg!(target_os = "linux") || std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() < 7 * 86_400)
                });
                match kept.and_then(|p| std::fs::read_to_string(p).ok()) {
                    Some(t) => t,
                    None if cfg!(target_os = "linux") => String::new(),
                    None => {
                        let t = scan();
                        if let Some(p) = index_file() {
                            let tmp = p.with_extension("tmp");
                            if std::fs::write(&tmp, &t).is_ok() {
                                let _ = std::fs::rename(&tmp, &p);
                            }
                        }
                        t
                    }
                }
            }
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

    #[test]
    fn scan_reads_family_weight_and_style() {
        let Some(p) = ["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"].iter().map(std::path::Path::new).find(|p| p.exists()) else { return };
        let mut out = String::new();
        describe(p, &std::fs::read(p).unwrap(), &mut out);
        let idx = parse(&out);
        let faces = &idx.families["dejavu sans"];
        assert_eq!((faces[0].weight, faces[0].italic, faces[0].index), (700, false, 0), "{out}");
    }
}
