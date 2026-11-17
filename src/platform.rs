//! What differs between Linux, Windows and macOS, in one place: the home and profile folders,
//! font folders, memory readings and stopping a child process. Everything else in Ironswift
//! is the same on every system.

use std::path::PathBuf;

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return Some(PathBuf::from(h));
    }
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Where applications keep their data: `$XDG_DATA_HOME` or `~/.local/share` on Linux,
/// `%APPDATA%` on Windows, `~/Library/Application Support` on macOS.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| home_dir().map(|h| h.join("AppData").join("Roaming")))
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library").join("Application Support"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home_dir().map(|h| h.join(".local").join("share")))
    }
}

/// Folders that hold installed fonts, system-wide first.
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        let win = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        dirs.push(win.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.extend(["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"].map(PathBuf::from));
        if let Some(h) = home_dir() {
            dirs.push(h.join("Library").join("Fonts"));
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(h) = home_dir() {
            dirs.push(h.join(".local").join("share").join("fonts"));
            dirs.push(h.join(".fonts"));
        }
    }
    dirs
}

/// Fonts shipped next to the program (`fonts/` beside the executable, as the Windows and
/// macOS packages do; on macOS also the app bundle's `Resources/fonts`).
pub fn bundled_font_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
        out.push(dir.join("fonts"));
        out.push(dir.join("..").join("Resources").join("fonts"));
    }
    out
}

/// The built-in faces (sans regular, bold, italic, bold italic, mono, mono bold) to use when
/// DejaVu isn't installed: each system's own equivalents, as (folder, file) pairs to try.
pub fn base_font_alternatives(slot: usize) -> Vec<PathBuf> {
    #[allow(unused_mut)]
    let mut out: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        const NAMES: [&[&str]; 6] = [
            &["arial.ttf", "segoeui.ttf"],
            &["arialbd.ttf", "segoeuib.ttf"],
            &["ariali.ttf", "segoeuii.ttf"],
            &["arialbi.ttf", "segoeuiz.ttf"],
            &["consola.ttf", "cour.ttf"],
            &["consolab.ttf", "courbd.ttf"],
        ];
        for d in font_dirs() {
            out.extend(NAMES[slot].iter().map(|n| d.join(n)));
        }
    }
    #[cfg(target_os = "macos")]
    {
        const NAMES: [&[&str]; 6] = [
            &["Supplemental/Arial.ttf", "Supplemental/Verdana.ttf"],
            &["Supplemental/Arial Bold.ttf", "Supplemental/Verdana Bold.ttf"],
            &["Supplemental/Arial Italic.ttf", "Supplemental/Verdana Italic.ttf"],
            &["Supplemental/Arial Bold Italic.ttf", "Supplemental/Verdana Bold Italic.ttf"],
            &["Supplemental/Courier New.ttf", "Supplemental/Andale Mono.ttf"],
            &["Supplemental/Courier New Bold.ttf", "Supplemental/Courier New.ttf"],
        ];
        out.extend(NAMES[slot].iter().map(|n| PathBuf::from("/System/Library/Fonts").join(n)));
    }
    let _ = slot;
    out
}

/// This process's resident memory in MiB (0 where the system doesn't say).
pub fn rss_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map_or(0, |pages| pages * 4 / 1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// The computer's (total, available) memory in MiB, where the system says.
pub fn memory_mb() -> Option<(u64, u64)> {
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        Some((crate::memory::meminfo(&info, "MemTotal")?, crate::memory::meminfo(&info, "MemAvailable")?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Stop a child process at once (it's stuck).
pub fn kill(pid: u32) {
    #[cfg(unix)]
    // SAFETY: sending a signal to our own child process.
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output();
    }
}

/// Open a file with its usual program, or a folder in the file manager.
pub fn open_path(path: &std::path::Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

/// A desktop notification where Ironswift knows how to show one without notify-send.
/// Returns false when it couldn't.
pub fn notify(title: &str, body: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("display notification \"{}\" with title \"{}\"", q(body), q(title));
        std::process::Command::new("osascript").args(["-e", &script]).spawn().is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, body);
        false
    }
}

/// The system's own file chooser on Windows (PowerShell) and macOS (AppleScript); one chosen
/// path per output line. None on Linux, which uses zenity or kdialog.
pub fn file_chooser(multiple: bool, filters: &[String], title: &str) -> Option<std::process::Command> {
    #[cfg(windows)]
    {
        let q = |s: &str| s.replace('\'', "''");
        let filter = if filters.is_empty() { "All files|*.*".to_string() } else { format!("Accepted files|{}|All files|*.*", filters.join(";")) };
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; \
             $d.Title = '{}'; $d.Filter = '{}'; $d.Multiselect = ${}; \
             if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ $d.FileNames -join \"`n\" }}",
            q(title),
            q(&filter),
            multiple
        );
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-STA", "-Command", &script]);
        Some(c)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = filters;
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let many = if multiple { " with multiple selections allowed" } else { "" };
        let script = format!(
            "set out to \"\"\nset picked to (choose file with prompt \"{}\"{many})\nif class of picked is not list then set picked to {{picked}}\nrepeat with f in picked\nset out to out & POSIX path of f & linefeed\nend repeat\nreturn out",
            q(title)
        );
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", &script]);
        Some(c)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (multiple, filters, title);
        None
    }
}
