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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // A Windows toast. The text comes from a website: it reaches the script through the
        // environment and is XML-escaped there, never pasted into the script.
        const SCRIPT: &str = r#"
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$t = [Security.SecurityElement]::Escape($env:IRONSWIFT_TOAST_TITLE)
$b = [Security.SecurityElement]::Escape($env:IRONSWIFT_TOAST_BODY)
$x = New-Object Windows.Data.Xml.Dom.XmlDocument
$x.LoadXml("<toast><visual><binding template='ToastGeneric'><text>$t</text><text>$b</text></binding></visual></toast>")
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($x))
"#;
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
            .env("IRONSWIFT_TOAST_TITLE", title)
            .env("IRONSWIFT_TOAST_BODY", body)
            .creation_flags(0x0800_0000)
            .spawn()
            .is_ok()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
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
        use std::os::windows::process::CommandExt;
        // The file types come from the page's `accept`: they reach the script through the
        // environment, never pasted into it.
        let filter = if filters.is_empty() { "All files|*.*".to_string() } else { format!("Accepted files|{}|All files|*.*", filters.join(";").replace('|', "")) };
        const SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; \
             $d.Title = $env:IRONSWIFT_PICK_TITLE; $d.Filter = $env:IRONSWIFT_PICK_FILTER; $d.Multiselect = ($env:IRONSWIFT_PICK_MANY -eq '1'); \
             if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $d.FileNames -join \"`n\" }";
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
            .env("IRONSWIFT_PICK_TITLE", title)
            .env("IRONSWIFT_PICK_FILTER", filter)
            .env("IRONSWIFT_PICK_MANY", if multiple { "1" } else { "0" })
            .creation_flags(0x0800_0000);
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

/// A `file:` URL for a local path, absolute and with the system's quirks removed: on Windows,
/// `D:\a\b.html` becomes `file:///D:/a/b.html` (never `file://\\?\D:\…`).
pub fn file_url(path: &std::path::Path) -> String {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        // canonicalize gives verbatim paths (\\?\D:\…), which file URLs can't hold.
        let s = abs.to_string_lossy();
        let plain = s.strip_prefix(r"\\?\UNC\").map(|r| format!(r"\\{r}")).or_else(|| s.strip_prefix(r"\\?\").map(String::from)).unwrap_or_else(|| s.to_string());
        url::Url::from_file_path(&plain).map(String::from).unwrap_or_else(|_| format!("file:///{}", plain.replace('\\', "/")))
    }
    #[cfg(not(windows))]
    {
        format!("file://{}", abs.display())
    }
}

/// The local path in a `file:` URL's (percent-decoded) path part: `/D:/a/b.html` is
/// `D:\a\b.html` on Windows.
pub fn path_from_url(path: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let b = path.as_bytes();
        let p = if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' { &path[1..] } else { path };
        std::path::PathBuf::from(p.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_round_trip() {
        let here = std::env::current_dir().unwrap().join("Cargo.toml");
        let u = file_url(&here);
        assert!(u.starts_with("file:///"), "{u}");
        assert!(!u.contains('\\'), "{u}");
        let back = path_from_url(&crate::net::percent_decode_str(u.strip_prefix("file://").unwrap()));
        assert_eq!(std::fs::canonicalize(back).unwrap(), std::fs::canonicalize(&here).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_paths() {
        assert_eq!(path_from_url("/D:/a/b c.html"), std::path::PathBuf::from(r"D:\a\b c.html"));
    }
}

/// Run a PowerShell script on Windows with extra environment variables (values pass through
/// the environment, never through the script's text, so nothing in them is ever run).
#[cfg(windows)]
pub fn powershell(script: &str, env: &[(&str, &str)]) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = std::process::Command::new("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]).creation_flags(CREATE_NO_WINDOW);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().ok()
}

/// Send a PDF to the default printer. False when the system has no way to (the caller then
/// saves the PDF instead).
pub fn print_pdf(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        let ok = powershell("Start-Process -FilePath $env:IRONSWIFT_PDF -Verb Print -ErrorAction Stop", &[("IRONSWIFT_PDF", &path.to_string_lossy())]).is_some_and(|o| o.status.success());
        if !ok {
            // No program offers "Print" for PDFs: open it, where the user can print it.
            open_path(path);
        }
        ok
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("lp").arg(path).output().is_ok_and(|o| o.status.success())
    }
}

/// Offer Ironswift as the default browser. Ok(message) to show; Err(why not).
pub fn make_default_browser() -> Result<String, String> {
    #[cfg(windows)]
    {
        // Windows lets the user, not the program, choose the default: register Ironswift as
        // a browser for this user (no administrator rights needed), then open Default apps.
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" \"%1\"", exe.display());
        let caps = r"HKCU\Software\Clients\StartMenuInternet\Ironswift\Capabilities";
        let entries: Vec<Vec<String>> = vec![
            vec![caps.into(), "ApplicationName".into(), "Ironswift".into()],
            vec![caps.into(), "ApplicationDescription".into(), "A web browser built from scratch".into()],
            vec![format!(r"{caps}\URLAssociations"), "http".into(), "IronswiftURL".into()],
            vec![format!(r"{caps}\URLAssociations"), "https".into(), "IronswiftURL".into()],
            vec![format!(r"{caps}\FileAssociations"), ".html".into(), "IronswiftHTML".into()],
            vec![format!(r"{caps}\FileAssociations"), ".htm".into(), "IronswiftHTML".into()],
            vec![r"HKCU\Software\Clients\StartMenuInternet\Ironswift\shell\open\command".into(), String::new(), format!("\"{}\"", exe.display())],
            vec![r"HKCU\Software\Classes\IronswiftURL".into(), String::new(), "Ironswift web page".into()],
            vec![r"HKCU\Software\Classes\IronswiftURL".into(), "URL Protocol".into(), String::new()],
            vec![r"HKCU\Software\Classes\IronswiftURL\shell\open\command".into(), String::new(), cmd.clone()],
            vec![r"HKCU\Software\Classes\IronswiftHTML".into(), String::new(), "Ironswift HTML document".into()],
            vec![r"HKCU\Software\Classes\IronswiftHTML\shell\open\command".into(), String::new(), cmd],
            vec![r"HKCU\Software\RegisteredApplications".into(), "Ironswift".into(), r"Software\Clients\StartMenuInternet\Ironswift\Capabilities".into()],
        ];
        for e in &entries {
            let mut c = std::process::Command::new("reg");
            c.args(["add", &e[0], "/f", "/t", "REG_SZ", "/d", &e[2]]);
            if e[1].is_empty() {
                c.arg("/ve");
            } else {
                c.args(["/v", &e[1]]);
            }
            let ok = c.output().is_ok_and(|o| o.status.success());
            if !ok {
                return Err(format!("couldn't register Ironswift with Windows ({})", e[0]));
            }
        }
        let _ = std::process::Command::new("explorer").arg("ms-settings:defaultapps?registeredAppUser=Ironswift").spawn();
        Ok("Ironswift is registered as a browser. In the Default apps window that opened, choose Ironswift.".into())
    }
    #[cfg(not(windows))]
    {
        let ok = std::process::Command::new("xdg-settings").args(["set", "default-web-browser", "ironswift.desktop"]).output().is_ok_and(|o| o.status.success());
        if ok { Ok("Ironswift is now your default browser.".into()) } else { Err("install Ironswift first (packaging/linux/install.sh), then try again".into()) }
    }
}

/// One position from Windows' location service: (latitude, longitude, accuracy in metres,
/// altitude, speed, heading). Needs location turned on in Windows' privacy settings.
#[cfg(windows)]
pub fn windows_location(high: bool) -> Result<(f64, f64, f64, Option<f64>, Option<f64>, Option<f64>), String> {
    let script = r#"
Add-Type -AssemblyName System.Device
$acc = if ($env:IRONSWIFT_GEO_HIGH -eq '1') { [System.Device.Location.GeoPositionAccuracy]::High } else { [System.Device.Location.GeoPositionAccuracy]::Default }
$w = New-Object System.Device.Location.GeoCoordinateWatcher($acc)
if (-not $w.TryStart($false, [TimeSpan]::FromSeconds(10))) { Write-Output 'ERR could not start'; exit }
$t = 0
while ($w.Status -ne 'Ready' -and $t -lt 100) { Start-Sleep -Milliseconds 100; $t++ }
if ($w.Permission -eq 'Denied') { Write-Output 'ERR denied'; exit }
$c = $w.Position.Location
if ($c.IsUnknown) { Write-Output 'ERR unknown'; exit }
$inv = [Globalization.CultureInfo]::InvariantCulture
Write-Output ('OK ' + (@($c.Latitude, $c.Longitude, $c.HorizontalAccuracy, $c.Altitude, $c.Speed, $c.Course) | ForEach-Object { $_.ToString($inv) }) -join ' ')
"#;
    let out = powershell(script, &[("IRONSWIFT_GEO_HIGH", if high { "1" } else { "0" })]).ok_or("Windows' location service isn't available")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.starts_with("OK ") || l.starts_with("ERR ")).unwrap_or("ERR no answer");
    if let Some(e) = line.strip_prefix("ERR ") {
        return Err(match e.trim() {
            "denied" => "Windows doesn't allow Ironswift to use your location (Settings → Privacy → Location)".into(),
            other => format!("No location from Windows ({other})"),
        });
    }
    let n: Vec<f64> = line[3..].split_whitespace().map(|v| v.parse().unwrap_or(f64::NAN)).collect();
    let opt = |v: Option<&f64>| v.copied().filter(|x| x.is_finite());
    match (n.first(), n.get(1), n.get(2)) {
        (Some(&la), Some(&lo), Some(&ac)) if la.is_finite() && lo.is_finite() => Ok((la, lo, if ac.is_finite() { ac } else { 1000.0 }, opt(n.get(3)), opt(n.get(4)), opt(n.get(5)))),
        _ => Err("No location from Windows".into()),
    }
}
