//! The app icon (the iron bird), built into the program from assets/icon/png.

/// Show the icon on the window (taskbar, Alt+Tab). X11 takes it at run time; on Wayland the
/// icon comes from the installed desktop file instead (see packaging/linux).
pub fn set_window_icon(window: &mut minifb::Window) {
    #[cfg(target_os = "linux")]
    {
        use minifb::HasWindowHandle;
        use raw_window_handle::RawWindowHandle;
        let x11 = window
            .window_handle()
            .is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)));
        if !x11 {
            return;
        }
        // _NET_WM_ICON: for each size, width, height, then ARGB pixels (one per 32-bit "long").
        let mut data: Vec<u64> = Vec::new();
        for bytes in [
            &include_bytes!("../assets/icon/png/ironswift-32.png")[..],
            &include_bytes!("../assets/icon/png/ironswift-64.png")[..],
            &include_bytes!("../assets/icon/png/ironswift-128.png")[..],
        ] {
            let Ok(img) = image::load_from_memory(bytes) else { continue };
            let img = img.to_rgba8();
            data.push(img.width() as u64);
            data.push(img.height() as u64);
            data.extend(img.pixels().map(|p| {
                let [r, g, b, a] = p.0;
                ((a as u64) << 24) | ((r as u64) << 16) | ((g as u64) << 8) | b as u64
            }));
        }
        if let Ok(icon) = minifb::Icon::try_from(&data[..]) {
            window.set_icon(icon);
        }
    }
}
