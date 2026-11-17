//! The page side of image loading: lazy images that come near the window, and animated
//! images that need their next frame. Kept out of main.rs's window loop, which calls these.

use crate::{images, js, layout::Layout, Page};

/// What the window loop should do after `Page::images_tick`.
#[derive(Default, Debug, PartialEq)]
pub struct ImagesTick {
    /// Lazy images arrived: lay the page out again.
    pub relayout: bool,
    /// An animation moved on: paint again (the page tiles are stale).
    pub repaint: bool,
}

impl Page {
    /// Lazy images within reach of the visible part (`top`..`bottom`, layout coordinates).
    fn lazy_in_view(&self, lay: &Layout, top: f32, bottom: f32) -> Vec<String> {
        let boxes = |id: usize| {
            lay.boxes.get(&id).map(|(r, _)| (r.y, r.y + r.h)).or_else(|| lay.hits.iter().find(|(_, n)| *n == id).map(|(r, _)| (r.y, r.y + r.h)))
        };
        js::with_dom(|d| images::lazy_wanted(d, &self.images, &boxes, (top, bottom)))
    }

    /// Called every frame: start loading lazy images that scrolled near, take in those that
    /// arrived, and notice animation frames that are due.
    pub(crate) fn images_tick(&mut self, lay: &Layout, scroll: f32, view_h: f32) -> ImagesTick {
        let mut t = ImagesTick::default();
        let wanted = self.lazy_in_view(lay, scroll, scroll + view_h);
        if !wanted.is_empty() {
            let url = self.url.clone();
            self.images.load_lazy(&wanted, &url);
        }
        t.relayout = self.images.poll_lazy();
        t.repaint = self.images.frame_due();
        t
    }

    /// For snapshots (--png): load the lazy images a window of this height would show, and
    /// wait for them. Returns true if any arrived.
    pub(crate) fn load_lazy_now(&mut self, lay: &Layout, view_h: f32) -> bool {
        let wanted = self.lazy_in_view(lay, 0.0, view_h);
        if wanted.is_empty() {
            return false;
        }
        let url = self.url.clone();
        self.images.load_lazy(&wanted, &url);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut any = false;
        while self.images.waiting_count() > 0 && std::time::Instant::now() < deadline {
            any |= self.images.poll_lazy();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        any | self.images.poll_lazy()
    }
}
