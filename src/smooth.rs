//! Smooth scrolling: arrow keys, Page Up/Down, Home/End and the mouse wheel glide to their
//! target over a few frames (about 150 ms) instead of jumping. Each frame moves part of the
//! remaining distance, so repeated presses speed up rather than queue.
//! `IRONSWIFT_SMOOTH_SCROLL=0` turns it off.

use std::time::{Duration, Instant};

/// How quickly the position closes in on the target (seconds for ~63% of the way).
const TAU: f32 = 0.045;

pub struct Smooth {
    target: Option<f32>,
    /// Where the last step left the page, to notice when something else scrolled it.
    last_set: f32,
    last: Instant,
    enabled: bool,
}

impl Default for Smooth {
    fn default() -> Self {
        Smooth { target: None, last_set: 0.0, last: Instant::now(), enabled: std::env::var("IRONSWIFT_SMOOTH_SCROLL").map_or(true, |v| v != "0") }
    }
}

impl Smooth {
    /// Scroll by `delta` from wherever the glide is heading (or `current`).
    pub fn by(&mut self, current: f32, delta: f32) -> f32 {
        let base = self.target.filter(|_| self.last_set == current).unwrap_or(current);
        self.to(current, base + delta)
    }

    /// Glide to `y`. Returns the position to use now (`y` itself when smoothing is off).
    pub fn to(&mut self, current: f32, y: f32) -> f32 {
        if !self.enabled {
            self.target = None;
            return y;
        }
        if self.target.is_none() || self.last_set != current {
            // Starting fresh: count from one frame ago, not from the last glide.
            self.last = Instant::now() - Duration::from_millis(16);
            self.last_set = current;
        }
        self.target = Some(y);
        current
    }

    /// Is a glide under way?
    pub fn moving(&self) -> bool {
        self.target.is_some()
    }

    /// This frame's position, if gliding. `max` is the furthest the page can scroll.
    pub fn step(&mut self, current: f32, max: f32) -> Option<f32> {
        let target = self.target?.clamp(0.0, max.max(0.0));
        if current != self.last_set {
            // Something else moved the page (a link to #anchor, a script): stop gliding.
            self.target = None;
            return None;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        let k = 1.0 - (-dt / TAU).exp();
        let mut y = current + (target - current) * k;
        if (target - y).abs() < 0.5 {
            y = target;
            self.target = None;
        }
        self.last_set = y;
        Some(y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glides_to_the_target() {
        let mut s = Smooth { enabled: true, ..Default::default() };
        let now = s.by(0.0, 100.0);
        assert_eq!(now, 0.0);
        let mut y = now;
        let mut frames = 0;
        while let Some(n) = s.step(y, 1000.0) {
            assert!(n >= y && n <= 100.0);
            y = n;
            frames += 1;
            std::thread::sleep(Duration::from_millis(16));
            assert!(frames < 60);
        }
        assert_eq!(y, 100.0);
        assert!(frames > 2, "it took {frames} frames");
        // Another press while gliding adds to the target.
        let mut s = Smooth { enabled: true, ..Default::default() };
        s.by(0.0, 100.0);
        let y = s.step(0.0, 1000.0).unwrap();
        s.by(y, 100.0);
        assert_eq!(s.target, Some(200.0));
        // Something else scrolling the page stops the glide.
        assert_eq!(s.step(500.0, 1000.0), None);
        assert!(!s.moving());
    }

    #[test]
    fn off_means_jump() {
        let mut s = Smooth { enabled: false, ..Default::default() };
        assert_eq!(s.by(10.0, 50.0), 60.0);
        assert!(!s.moving());
    }
}
