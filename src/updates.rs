//! Automatic updates. Once a day Ironswift asks GitHub for the newest release. When there is
//! a newer version:
//!
//! - If Ironswift was installed for your user (`packaging/linux/install.sh`, so the program
//!   file is yours to replace), the new program is downloaded, checked against the SHA-256
//!   checksum published with the release, and put in place when Ironswift closes. The next
//!   start runs the new version.
//! - Otherwise (a system or Flatpak install, or a build in a source folder) about:settings and
//!   the start page say a new version is out; the package manager or `git pull` updates it.
//!
//! Release files (made by `.github/workflows/release.yml`): `ironswift-linux-x86_64` and
//! `ironswift-linux-x86_64.sha256`.

use std::sync::Mutex;

pub const REPO: &str = "hirejohndougan-art/lighteningfox";
const ASSET: &str = "ironswift-linux-x86_64";

#[derive(Clone, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    NotChecked,
    UpToDate,
    /// A newer version exists (version, release page).
    Available(String, String),
    /// Downloaded and verified; installed when Ironswift closes.
    Ready(String),
    Failed(String),
}

static STATE: Mutex<State> = Mutex::new(State::NotChecked);

fn set_state(s: State) {
    if let Ok(mut g) = STATE.lock() {
        *g = s;
    }
}

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

/// "1.2.10" -> [1, 2, 10] (a leading "v" and anything after "-" are ignored).
fn parse_version(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

pub fn newer(candidate: &str, current: &str) -> bool {
    let (a, b) = (parse_version(candidate), parse_version(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// The value of `"key": "…"` in JSON text (enough for GitHub's release format).
fn json_str<'a>(json: &'a str, key: &str, from: usize) -> Option<(&'a str, usize)> {
    let pat = format!("\"{key}\"");
    let i = json[from..].find(&pat)? + from + pat.len();
    let rest = &json[i..];
    let q = rest.find('"')? + 1;
    let end = rest[q..].find('"')? + q;
    Some((&rest[q..end], i + end))
}

/// (version, release page, program download URL, checksum URL) of the newest release.
fn latest() -> Result<(String, String, Option<String>, Option<String>), String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let headers = [("Accept".to_string(), "application/vnd.github+json".to_string())];
    let req = crate::net::Request { method: "GET", url: &url, headers: &headers, body: None, top: None, cookies: false, extra: &[] };
    let r = crate::net::http(&req)?;
    if r.status == 404 {
        return Err("no releases published yet".into());
    }
    if r.status != 200 {
        return Err(format!("GitHub answered {}", r.status));
    }
    let json = String::from_utf8_lossy(&r.body).to_string();
    let tag = json_str(&json, "tag_name", 0).ok_or("no version in the answer")?.0.to_string();
    let page = json_str(&json, "html_url", 0).map(|v| v.0.to_string()).unwrap_or_default();
    let mut bin = None;
    let mut sum = None;
    let mut at = 0;
    while let Some((u, next)) = json_str(&json, "browser_download_url", at) {
        if u.ends_with(&format!("/{ASSET}")) {
            bin = Some(u.to_string());
        } else if u.ends_with(&format!("/{ASSET}.sha256")) {
            sum = Some(u.to_string());
        }
        at = next;
    }
    Ok((tag, page, bin, sum))
}

/// Can this copy of Ironswift replace its own program file? Only a per-user install
/// (~/.local/bin and the like), never a system, Flatpak or development build. Only on Linux:
/// the release's program file is the Linux one, so Windows and macOS copies just say that a
/// new version is out.
fn self_updatable() -> Option<std::path::PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_string_lossy();
    if s.contains("/target/") || s.starts_with("/usr/") || s.starts_with("/app/") || std::env::var("FLATPAK_ID").is_ok() {
        return None;
    }
    let home = crate::platform::home_dir()?.to_string_lossy().into_owned();
    if !s.starts_with(&home) {
        return None;
    }
    let dir = exe.parent()?;
    let probe = dir.join(".ironswift-write-test");
    std::fs::write(&probe, b"").ok()?;
    let _ = std::fs::remove_file(&probe);
    Some(exe)
}

fn staged_path() -> Option<std::path::PathBuf> {
    crate::cookies::profile_dir().map(|d| d.join("update").join(ASSET))
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let req = crate::net::Request { method: "GET", url, headers: &[], body: None, top: None, cookies: false, extra: &[] };
    let r = crate::net::http(&req)?;
    if r.status != 200 {
        return Err(format!("{url}: HTTP {}", r.status));
    }
    Ok(r.body)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Check now (and download, if this install can update itself). Returns a sentence for the
/// settings page.
pub fn check_now() -> String {
    let current = env!("CARGO_PKG_VERSION");
    let result = (|| -> Result<State, String> {
        let (tag, page, bin, sum) = latest()?;
        if !newer(&tag, current) {
            return Ok(State::UpToDate);
        }
        let (Some(exe), Some(bin), Some(sum)) = (self_updatable(), bin, sum) else { return Ok(State::Available(tag, page)) };
        let _ = exe;
        let data = get_bytes(&bin)?;
        let want = String::from_utf8_lossy(&get_bytes(&sum)?).split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if want.len() != 64 || sha256_hex(&data) != want {
            return Err("the download didn't match its published checksum, so it wasn't used".into());
        }
        if !data.starts_with(b"\x7fELF") {
            return Err("the download isn't a Linux program".into());
        }
        let path = staged_path().ok_or("no profile folder")?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, &data).map_err(|e| e.to_string())?;
        std::fs::write(path.with_extension("version"), &tag).map_err(|e| e.to_string())?;
        Ok(State::Ready(tag))
    })();
    let state = result.unwrap_or_else(State::Failed);
    if let Some(f) = crate::cookies::profile_dir().map(|d| d.join("update-checked")) {
        let _ = std::fs::write(f, b"");
    }
    set_state(state);
    status()
}

/// One line describing the update state.
pub fn status() -> String {
    let current = env!("CARGO_PKG_VERSION");
    match state() {
        State::NotChecked => format!("You have version {current}."),
        State::UpToDate => format!("Version {current} is the newest."),
        State::Available(v, _) => format!("Version {v} is available (you have {current}). Update it with your package manager, or rebuild from the repository."),
        State::Ready(v) => format!("Version {v} has been downloaded and checked; it's installed when you close Ironswift."),
        State::Failed(e) => format!("You have version {current}. The last update check failed: {e}."),
    }
}

/// Start the daily check (a minute after startup, so it never slows the first page).
pub fn start() {
    if std::env::var("IRONSWIFT_NO_UPDATES").is_ok_and(|v| v == "1") || cfg!(test) || !crate::settings::get().check_updates {
        return;
    }
    let due = crate::cookies::profile_dir()
        .and_then(|d| std::fs::metadata(d.join("update-checked")).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age.as_secs() > 24 * 3600);
    if !due {
        return;
    }
    let _ = std::thread::Builder::new().name("updates".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        log!("[update] {}", check_now());
    });
}

/// At exit: put a downloaded, verified update in place of the running program.
pub fn install_staged() {
    let (Some(staged), Some(exe)) = (staged_path(), self_updatable()) else { return };
    if !staged.exists() {
        return;
    }
    let tmp = exe.with_extension("new");
    let ok = std::fs::copy(&staged, &tmp).is_ok() && {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
        }
        // Renaming over the running program is safe on Linux: it keeps running from the old file.
        std::fs::rename(&tmp, &exe).is_ok()
    };
    if ok {
        let _ = std::fs::remove_file(&staged);
        log!("[update] installed; the next start runs the new version");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v0.2.0", "0.1.0"));
        assert!(newer("0.1.10", "0.1.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("v0.0.9", "0.1.0"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn release_json() {
        let j = r#"{"tag_name": "v0.2.0", "html_url": "https://github.com/x/y/releases/tag/v0.2.0",
          "assets": [{"browser_download_url": "https://github.com/x/y/releases/download/v0.2.0/ironswift-linux-x86_64.sha256"},
                     {"browser_download_url": "https://github.com/x/y/releases/download/v0.2.0/ironswift-linux-x86_64"}]}"#;
        assert_eq!(json_str(j, "tag_name", 0).unwrap().0, "v0.2.0");
        let (first, next) = json_str(j, "browser_download_url", 0).unwrap();
        assert!(first.ends_with(".sha256"));
        assert!(json_str(j, "browser_download_url", next).unwrap().0.ends_with("/ironswift-linux-x86_64"));
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
