//! Page zoom, remembered per site (like other browsers): Ctrl +, Ctrl −, Ctrl 0, Ctrl+wheel.
//! Saved in `<profile>/zoom.txt` as `host factor` lines.

use std::collections::HashMap;
use std::sync::Mutex;

/// The steps Ctrl + and Ctrl − move through (as in Chrome and Firefox).
pub const LEVELS: [f32; 17] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];

static ZOOMS: Mutex<Option<HashMap<String, f32>>> = Mutex::new(None);

fn key(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).to_string())
}

fn file() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("zoom.txt"))
}

fn with<R>(f: impl FnOnce(&mut HashMap<String, f32>) -> R) -> R {
    let mut g = ZOOMS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(|| {
        let mut m = HashMap::new();
        if let Some(text) = file().and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                if let Some((host, z)) = line.rsplit_once(' ') {
                    if let Ok(z) = z.parse::<f32>() {
                        m.insert(host.to_string(), z.clamp(LEVELS[0], LEVELS[LEVELS.len() - 1]));
                    }
                }
            }
        }
        m
    });
    f(map)
}

/// The zoom factor for a page (1.0 unless the site was zoomed).
pub fn get(url: &str) -> f32 {
    let k = key(url);
    with(|m| m.get(&k).copied().unwrap_or(1.0))
}

pub fn set(url: &str, z: f32) {
    let k = key(url);
    let text = with(|m| {
        if (z - 1.0).abs() < 1e-3 {
            m.remove(&k);
        } else {
            m.insert(k, z);
        }
        let mut lines: Vec<String> = m.iter().map(|(h, z)| format!("{h} {z}")).collect();
        lines.sort();
        lines.join("\n")
    });
    if std::env::var("IRONSWIFT_PRIVATE").is_ok_and(|v| v == "1") {
        return;
    }
    if let Some(f) = file() {
        let _ = std::fs::write(f, text);
    }
}

/// The next level in or out from `z`.
pub fn step(z: f32, zoom_in: bool) -> f32 {
    if zoom_in {
        LEVELS.iter().copied().find(|&l| l > z + 1e-3).unwrap_or(LEVELS[LEVELS.len() - 1])
    } else {
        LEVELS.iter().rev().copied().find(|&l| l < z - 1e-3).unwrap_or(LEVELS[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps() {
        assert_eq!(step(1.0, true), 1.1);
        assert_eq!(step(1.0, false), 0.9);
        assert_eq!(step(5.0, true), 5.0);
        assert_eq!(step(0.25, false), 0.25);
        assert_eq!(key("https://example.com/a?b"), "example.com");
    }
}
