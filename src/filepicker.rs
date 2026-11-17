//! The file chooser for `<input type=file>`: the desktop's own open-file dialog (zenity,
//! kdialog, yad or qarma, whichever is installed), run on a helper thread so the window keeps
//! drawing while it is open.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use crate::dom::NodeId;

/// An open dialog, waiting for the user.
pub struct Pending {
    pub node: NodeId,
    /// The document the input belongs to (a navigation in the meantime drops the result).
    pub doc: u64,
    rx: Receiver<Option<Vec<PathBuf>>>,
}

impl Pending {
    /// `Some(None)`: closed without choosing. `Some(Some(files))`: chosen. `None`: still open.
    pub fn poll(&self) -> Option<Option<Vec<PathBuf>>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    }
}

/// The dialog programs we know, in the order we try them.
const DIALOGS: &[&str] = &["zenity", "kdialog", "yad", "qarma"];

fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// Shell-style patterns for an `accept` attribute (".png,image/*" → "*.png *.jpg …").
pub fn patterns(accept: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in accept.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
        let exts: &[&str] = match item.as_str() {
            "image/*" => &["png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "bmp", "ico"],
            "audio/*" => &["mp3", "ogg", "oga", "wav", "flac", "m4a", "opus"],
            "video/*" => &["mp4", "webm", "mkv", "mov", "ogv"],
            "text/*" => &["txt", "csv", "html", "htm", "css", "md"],
            "image/jpeg" => &["jpg", "jpeg"],
            "application/pdf" => &["pdf"],
            "application/json" => &["json"],
            "application/zip" => &["zip"],
            "text/plain" => &["txt", "text"],
            "text/csv" => &["csv"],
            other => {
                if let Some(ext) = other.strip_prefix('.') {
                    out.push(format!("*.{ext}"));
                } else if let Some((_, sub)) = other.split_once('/') {
                    // image/png → *.png (the subtype is usually the extension).
                    if !sub.contains('*') && !sub.contains('.') && !sub.contains('+') {
                        out.push(format!("*.{sub}"));
                    }
                }
                &[]
            }
        };
        out.extend(exts.iter().map(|e| format!("*.{e}")));
    }
    out.dedup();
    out
}

/// The command line for one dialog program.
fn command(program: &str, multiple: bool, filters: &[String], title: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    let start = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    match program {
        "kdialog" => {
            c.arg("--title").arg(title).arg("--getopenfilename");
            if multiple {
                c.arg("--multiple").arg("--separate-output");
            }
            c.arg(&start);
            if !filters.is_empty() {
                c.arg(format!("{} | Accepted files", filters.join(" ")));
            }
        }
        _ => {
            c.arg("--file-selection").arg(format!("--title={title}")).arg(format!("--filename={start}/"));
            if multiple {
                c.arg("--multiple").arg("--separator=\n");
            }
            if !filters.is_empty() {
                c.arg(format!("--file-filter=Accepted files | {}", filters.join(" ")));
                c.arg("--file-filter=All files | *");
            }
        }
    }
    c.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c
}

/// Open the chooser. Err when no dialog program is installed.
pub fn open(node: NodeId, doc: u64, multiple: bool, accept: &str) -> Result<Pending, String> {
    let program = DIALOGS.iter().find(|d| find_program(d).is_some()).ok_or("No file chooser found (install zenity or kdialog)")?;
    let filters = patterns(accept);
    let title = if multiple { "Choose files to upload" } else { "Choose a file to upload" };
    let mut cmd = command(program, multiple, &filters, title);
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("file-chooser".into())
        .spawn(move || {
            let chosen = cmd.output().ok().filter(|o| o.status.success()).map(|o| {
                String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).filter(|p| p.is_file()).collect::<Vec<_>>()
            });
            let _ = tx.send(chosen.filter(|v| !v.is_empty()));
        })
        .map_err(|e| e.to_string())?;
    Ok(Pending { node, doc, rx })
}

/// What a file input shows next to its button.
pub fn label(names: &[String]) -> String {
    match names {
        [] => "No file selected.".into(),
        [one] => one.clone(),
        many => format!("{} files selected.", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accept_patterns() {
        assert_eq!(patterns(".PDF, image/png"), vec!["*.pdf", "*.png"]);
        assert!(patterns("image/*").contains(&"*.webp".to_string()));
        assert!(patterns("").is_empty());
        assert_eq!(label(&[]), "No file selected.");
        assert_eq!(label(&["a".into(), "b".into()]), "2 files selected.");
    }
}
