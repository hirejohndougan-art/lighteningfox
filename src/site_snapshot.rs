//! `--png` through a site process: with IRONSWIFT_SITE_PROCESS=1, a snapshot of an http(s)
//! page is taken the way the window shows it with site processes on — the page runs in its
//! site's sandboxed process (process.rs) and the picture it sends back is saved. Lets the
//! screenshot and site checks run against site processes without a window.

use crate::frames::Update;
use std::time::{Duration, Instant};

/// Should this snapshot go through a site process?
pub fn wanted(url: &str) -> bool {
    std::env::var("IRONSWIFT_SITE_PROCESS").is_ok_and(|v| v == "1") && crate::process::site_of(url).is_some()
}

/// Open `url` in its site's process at `width`×`height`, and save the last picture it sends
/// once it has stopped changing for `quiet` (or `max` has passed). False if no picture came.
pub fn snapshot(url: &str, width: u32, height: u32, out: &str, quiet: Duration, max: Duration) -> bool {
    crate::process::enable_from_settings();
    let (utx, urx) = std::sync::mpsc::channel::<Update>();
    let (reply, _) = std::sync::mpsc::channel();
    let job = crate::FrameJob {
        src: crate::FrameSrc::Url(url.to_string()),
        w: width,
        h: height,
        parent: url.to_string(),
        top: url.to_string(),
        sandbox: None,
        reply,
        top_level: true,
    };
    let Some(id) = crate::process::open(&job, utx) else {
        log!("[snapshot] couldn't open {url} in a site process");
        return false;
    };
    let start = Instant::now();
    let mut last: Option<image::RgbaImage> = None;
    let mut changed = Instant::now();
    while start.elapsed() < max {
        match urx.recv_timeout(Duration::from_millis(100)) {
            Ok(Update::Picture { img, .. }) => {
                if last.as_ref().is_none_or(|l| l.as_raw() != img.as_raw()) {
                    changed = Instant::now();
                }
                last = Some(img);
            }
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last.is_some() && changed.elapsed() >= quiet {
            break;
        }
    }
    crate::process::close(id);
    match last {
        Some(img) => image::DynamicImage::ImageRgba8(img).to_rgb8().save(out).map_err(|e| log!("[snapshot] {e}")).is_ok(),
        None => {
            log!("[snapshot] no picture from the site process for {url}");
            false
        }
    }
}
