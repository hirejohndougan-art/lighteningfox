//! CSS grid helpers: parsing track lists and areas, resolving item placement,
//! and sizing tracks (fixed, percentage, `fr`, `auto`, `minmax()`, `repeat()`).

use crate::css;
use crate::style::{parse_len, GridPlace, Len};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Max {
    Px(f32),
    Fr(f32),
    /// Sized by content (auto, min-content, max-content).
    Content,
}

/// One track: a minimum in px (`None` = its content's minimum) and a maximum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    pub min: Option<f32>,
    pub max: Max,
}

impl Track {
    pub const AUTO: Track = Track { min: None, max: Max::Content };

    pub fn needs_content(&self) -> bool {
        self.min.is_none() || self.max == Max::Content
    }
}

fn parse_size(t: &str, avail: f32, fs: f32) -> Option<Max> {
    let t = t.trim();
    if let Some(n) = t.strip_suffix("fr") {
        return n.trim().parse().ok().map(Max::Fr);
    }
    match t {
        "auto" | "min-content" | "max-content" => return Some(Max::Content),
        _ => {}
    }
    parse_len(t, fs).map(|l| match l {
        Len::Auto => Max::Content,
        l => Max::Px(l.resolve(avail)),
    })
}

fn parse_track(t: &str, avail: f32, fs: f32) -> Option<Track> {
    let t = t.trim();
    if let Some(inner) = t.strip_prefix("minmax(").and_then(|r| r.strip_suffix(')')) {
        let (a, b) = inner.split_once(',')?;
        let min = match parse_size(a, avail, fs)? {
            Max::Px(p) => Some(p),
            _ => None,
        };
        return Some(Track { min, max: parse_size(b, avail, fs)? });
    }
    if let Some(inner) = t.strip_prefix("fit-content(").and_then(|r| r.strip_suffix(')')) {
        return match parse_size(inner, avail, fs)? {
            Max::Px(p) => Some(Track { min: None, max: Max::Px(p) }),
            _ => Some(Track::AUTO),
        };
    }
    Some(match parse_size(t, avail, fs)? {
        Max::Px(p) => Track { min: Some(p), max: Max::Px(p) },
        Max::Fr(f) => Track { min: None, max: Max::Fr(f) },
        Max::Content => Track::AUTO,
    })
}

/// Parse a track list such as `200px 1fr`, `repeat(3, 1fr)` or
/// `repeat(auto-fill, minmax(180px, 1fr))` for a container `avail` px wide.
pub fn parse_tracks(s: &str, avail: f32, gap: f32, fs: f32) -> Vec<Track> {
    let mut out = Vec::new();
    let s = s.trim();
    if s.is_empty() || s == "none" {
        return out;
    }
    for tok in css::split_top(s, ' ') {
        let tok = tok.trim();
        if tok.is_empty() || tok.starts_with('[') {
            continue; // line names
        }
        if let Some(inner) = tok.strip_prefix("repeat(").and_then(|r| r.strip_suffix(')')) {
            let Some((count, list)) = inner.split_once(',') else { continue };
            let tracks: Vec<Track> = css::split_top(list, ' ').iter().filter(|t| !t.trim().is_empty() && !t.trim().starts_with('[')).filter_map(|t| parse_track(t, avail, fs)).collect();
            if tracks.is_empty() {
                continue;
            }
            let n = match count.trim() {
                "auto-fill" | "auto-fit" => {
                    // As many repetitions as fit, using each track's minimum (or fixed) size.
                    let one: f32 = tracks
                        .iter()
                        .map(|t| t.min.or(match t.max { Max::Px(p) => Some(p), _ => None }).unwrap_or(0.0))
                        .sum::<f32>()
                        + gap * tracks.len() as f32;
                    if one <= 0.0 {
                        1
                    } else {
                        (((avail + gap) / one).floor() as usize).max(1)
                    }
                }
                c => c.parse::<usize>().unwrap_or(1).clamp(1, 1000),
            };
            for _ in 0..n {
                out.extend(tracks.iter().copied());
            }
        } else if let Some(t) = parse_track(tok, avail, fs) {
            out.push(t);
        }
    }
    out
}

/// Named areas from `grid-template-areas: "head head" "nav main"`.
pub struct Areas {
    pub map: HashMap<String, (usize, usize, usize, usize)>, // row0, col0, row1 (excl), col1 (excl)
    pub rows: usize,
}

pub fn parse_areas(s: &str) -> Areas {
    let mut map: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut rows = 0;
    let mut rest = s;
    while let Some(start) = rest.find(['"', '\'']) {
        let q = rest.as_bytes()[start] as char;
        let after = &rest[start + 1..];
        let Some(end) = after.find(q) else { break };
        for (c, name) in after[..end].split_whitespace().enumerate() {
            if name.chars().all(|ch| ch == '.') {
                continue;
            }
            let e = map.entry(name.to_string()).or_insert((rows, c, rows + 1, c + 1));
            e.0 = e.0.min(rows);
            e.1 = e.1.min(c);
            e.2 = e.2.max(rows + 1);
            e.3 = e.3.max(c + 1);
        }
        rows += 1;
        rest = &after[end + 1..];
    }
    Areas { map, rows }
}

/// One axis of a placement like `2 / 4`, `span 2`, `1 / -1`, `sidebar`: (start index, span).
fn resolve_axis(spec: &str, explicit: usize, area: Option<(usize, usize)>) -> (Option<usize>, usize) {
    if let Some((s0, s1)) = area {
        return (Some(s0), s1 - s0);
    }
    let spec = spec.trim();
    if spec.is_empty() || spec == "auto" {
        return (None, 1);
    }
    let (a, b) = match spec.split_once('/') {
        Some((a, b)) => (a.trim(), Some(b.trim())),
        None => (spec, None),
    };
    let line = |t: &str| -> Option<usize> {
        let n: i64 = t.parse().ok()?;
        Some(if n > 0 { (n - 1) as usize } else { (explicit as i64 + 1 + n).max(0) as usize })
    };
    let span_of = |t: &str| t.strip_prefix("span").and_then(|n| n.trim().parse::<usize>().ok()).map(|n| n.max(1));
    match (a, b) {
        (a, None) => match (line(a), span_of(a)) {
            (Some(s), _) => (Some(s), 1),
            (None, Some(n)) => (None, n),
            _ => (None, 1),
        },
        (a, Some(b)) => {
            let start = line(a);
            match (start, line(b), span_of(b), span_of(a)) {
                (Some(s), Some(e), _, _) if e > s => (Some(s), e - s),
                (Some(s), Some(e), _, _) if e < s => (Some(e), s - e),
                (Some(s), _, Some(n), _) => (Some(s), n),
                (None, Some(e), _, Some(n)) => (Some(e.saturating_sub(n)), n),
                (Some(s), _, _, _) => (Some(s), 1),
                (None, _, Some(n), _) | (None, _, _, Some(n)) => (None, n),
                _ => (None, 1),
            }
        }
    }
}

/// ((column start, span), (row start, span)) for a grid item.
pub fn resolve_place(p: &GridPlace, areas: &Areas, cols: usize, rows: usize) -> ((Option<usize>, usize), (Option<usize>, usize)) {
    let mut col_spec = p.column.clone();
    let mut row_spec = p.row.clone();
    let mut area: Option<(usize, usize, usize, usize)> = None;
    let a = p.area.trim();
    if !a.is_empty() {
        if let Some(r) = areas.map.get(a) {
            area = Some(*r);
        } else {
            // grid-area: row-start / column-start / row-end / column-end
            let parts: Vec<&str> = a.split('/').map(str::trim).collect();
            row_spec = format!("{} / {}", parts.first().unwrap_or(&"auto"), parts.get(2).unwrap_or(&"auto"));
            col_spec = format!("{} / {}", parts.get(1).unwrap_or(&"auto"), parts.get(3).unwrap_or(&"auto"));
        }
    }
    // A single area name on grid-column/grid-row also works.
    let named = |spec: &str, rows_axis: bool| {
        areas.map.get(spec.trim()).map(|r| if rows_axis { (r.0, r.2) } else { (r.1, r.3) })
    };
    let c = resolve_axis(&col_spec, cols, area.map(|r| (r.1, r.3)).or_else(|| named(&col_spec, false)));
    let r = resolve_axis(&row_spec, rows, area.map(|r| (r.0, r.2)).or_else(|| named(&row_spec, true)));
    (c, r)
}

/// Track sizes for `avail` px: fixed tracks keep their size, content tracks take
/// their content, `fr` tracks share what's left. With `stretch`, content tracks grow
/// to fill leftover space when there are no `fr` tracks.
pub fn size_tracks(tracks: &[Track], content: &[(f32, f32)], avail: f32, gap: f32, stretch: bool) -> Vec<f32> {
    let n = tracks.len();
    let gaps = gap * n.saturating_sub(1) as f32;
    let mut size = vec![0.0f32; n];
    let mut fr_total = 0.0;
    for (i, t) in tracks.iter().enumerate() {
        let (cmin, cmax) = content.get(i).copied().unwrap_or((0.0, 0.0));
        size[i] = match t.max {
            Max::Px(p) => p.max(t.min.unwrap_or(0.0)).min(p.max(cmin)),
            Max::Fr(f) => {
                fr_total += f;
                t.min.unwrap_or(cmin)
            }
            Max::Content => t.min.unwrap_or(0.0).max(cmax),
        };
        if let Max::Px(p) = t.max {
            if t.min.is_none() {
                // fit-content(): content, capped
                size[i] = cmax.min(p).max(cmin);
            }
        }
    }
    let used: f32 = size.iter().sum::<f32>() + gaps;
    let mut left = avail - used;
    if fr_total > 0.0 && avail > 0.0 {
        // Share the space among fr tracks; a track whose share is below its minimum keeps the minimum.
        let mut flexible: Vec<usize> = (0..n).filter(|&i| matches!(tracks[i].max, Max::Fr(_))).collect();
        let base: f32 = flexible.iter().map(|&i| size[i]).sum();
        let mut space = left + base;
        for _ in 0..n + 1 {
            let total: f32 = flexible.iter().map(|&i| if let Max::Fr(f) = tracks[i].max { f } else { 0.0 }).sum();
            if total <= 0.0 {
                break;
            }
            let per = (space / total).max(0.0);
            let mut frozen = None;
            for &i in &flexible {
                if let Max::Fr(f) = tracks[i].max {
                    if per * f < size[i] {
                        frozen = Some(i);
                    }
                }
            }
            match frozen {
                Some(i) => {
                    space -= size[i];
                    flexible.retain(|&j| j != i);
                }
                None => {
                    for &i in &flexible {
                        if let Max::Fr(f) = tracks[i].max {
                            size[i] = per * f;
                        }
                    }
                    break;
                }
            }
        }
    } else if left > 0.0 && stretch {
        let autos: Vec<usize> = (0..n).filter(|&i| tracks[i].max == Max::Content).collect();
        if !autos.is_empty() {
            let each = left / autos.len() as f32;
            for i in autos {
                size[i] += each;
            }
        }
    } else if left < 0.0 {
        // Too wide: shrink content tracks toward their minimum content.
        for (i, t) in tracks.iter().enumerate() {
            if left >= 0.0 {
                break;
            }
            if t.max == Max::Content {
                let floor = t.min.unwrap_or(content.get(i).map_or(0.0, |c| c.0));
                let give = (size[i] - floor).max(0.0).min(-left);
                size[i] -= give;
                left += give;
            }
        }
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracks_and_placement() {
        let t = parse_tracks("200px 1fr 2fr", 800.0, 0.0, 16.0);
        let s = size_tracks(&t, &[(0.0, 0.0); 3], 800.0, 0.0, true);
        assert_eq!(s, vec![200.0, 200.0, 400.0]);
        let t = parse_tracks("repeat(auto-fill, minmax(180px, 1fr))", 800.0, 20.0, 16.0);
        assert_eq!(t.len(), 4);
        let a = parse_areas(r#""head head" "nav main" "foot foot""#);
        assert_eq!(a.map["head"], (0, 0, 1, 2));
        assert_eq!(a.map["main"], (1, 1, 2, 2));
        let p = GridPlace { column: "1 / -1".into(), ..Default::default() };
        assert_eq!(resolve_place(&p, &a, 3, 0).0, (Some(0), 3));
        let p = GridPlace { column: "span 2".into(), ..Default::default() };
        assert_eq!(resolve_place(&p, &a, 3, 0).0, (None, 2));
    }
}

