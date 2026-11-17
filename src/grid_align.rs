//! `align-content` for grid rows: when a grid is taller than its rows, the rows move as a
//! group (center, end) or spread out (space-between…) instead of the auto rows stretching.
//! BBC News (2026-11) centres its lead headline beside the picture with
//! `display: grid; align-content: center; height: 100%`.

use crate::style::Justify;

/// The `align-content` value: None for normal/stretch (rows stretch to fill), else the
/// distribution. Baseline values are treated as start.
pub fn parse_align_content(v: &str) -> Option<Justify> {
    let v = v.trim();
    let v = v.strip_prefix("safe ").or_else(|| v.strip_prefix("unsafe ")).unwrap_or(v).trim();
    match v {
        "center" => Some(Justify::Center),
        "start" | "flex-start" | "baseline" | "first baseline" | "last baseline" => Some(Justify::Start),
        "end" | "flex-end" => Some(Justify::End),
        "space-between" => Some(Justify::SpaceBetween),
        "space-around" => Some(Justify::SpaceAround),
        "space-evenly" => Some(Justify::SpaceEvenly),
        _ => None,
    }
}

/// Space before the first row and extra space between rows, for rows `heights` with `gap`
/// in a grid `definite_h` tall. No space is shared when the height isn't set or the rows
/// already fill it.
pub fn row_offsets(align: Option<Justify>, definite_h: Option<f32>, heights: &[f32], gap: f32) -> (f32, f32) {
    let (Some(align), Some(h)) = (align, definite_h) else { return (0.0, 0.0) };
    if heights.is_empty() {
        return (0.0, 0.0);
    }
    let used = heights.iter().sum::<f32>() + gap * (heights.len() - 1) as f32;
    let free = h - used;
    if free <= 0.0 {
        return (0.0, 0.0);
    }
    let n = heights.len() as f32;
    match align {
        Justify::Start => (0.0, 0.0),
        Justify::End => (free, 0.0),
        Justify::Center => (free / 2.0, 0.0),
        Justify::SpaceBetween if heights.len() > 1 => (0.0, free / (n - 1.0)),
        Justify::SpaceBetween => (0.0, 0.0),
        Justify::SpaceAround => (free / n / 2.0, free / n),
        Justify::SpaceEvenly => (free / (n + 1.0), free / (n + 1.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets() {
        assert_eq!(parse_align_content("safe center"), Some(Justify::Center));
        assert_eq!(parse_align_content("normal"), None);
        assert_eq!(row_offsets(Some(Justify::Center), Some(100.0), &[20.0, 20.0], 10.0), (25.0, 0.0));
        assert_eq!(row_offsets(Some(Justify::SpaceBetween), Some(100.0), &[20.0, 20.0], 10.0), (0.0, 50.0));
        assert_eq!(row_offsets(Some(Justify::End), None, &[20.0], 0.0), (0.0, 0.0));
        assert_eq!(row_offsets(Some(Justify::End), Some(10.0), &[20.0], 0.0), (0.0, 0.0));
    }
}
