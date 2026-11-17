//! HTML forms: what kind of control an element is, reading and changing its value,
//! and turning a form into the data a submission sends.

use crate::dom::{Dom, NodeData, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Single-line text: text, search, email, url, tel, number, password, date…
    Text { password: bool },
    TextArea,
    Checkbox,
    Radio,
    Select,
    /// Push buttons: <button>, input type=submit/reset/button/image.
    Button { submit: bool, reset: bool },
    /// Controls drawn as plain boxes for now (file, color, range).
    Other,
}

/// The control an element is, if it is one. Hidden inputs are not (they draw nothing).
pub fn control(dom: &Dom, id: NodeId) -> Option<Control> {
    let tag = dom.tag(id)?;
    let ty = || dom.attr(id, "type").unwrap_or("").trim().to_ascii_lowercase();
    Some(match tag {
        "input" => match ty().as_str() {
            "hidden" => return None,
            "checkbox" => Control::Checkbox,
            "radio" => Control::Radio,
            "submit" | "image" => Control::Button { submit: true, reset: false },
            "reset" => Control::Button { submit: false, reset: true },
            "button" => Control::Button { submit: false, reset: false },
            "file" | "color" | "range" => Control::Other,
            "password" => Control::Text { password: true },
            _ => Control::Text { password: false },
        },
        "textarea" => Control::TextArea,
        "select" => Control::Select,
        "button" => {
            let t = ty();
            Control::Button { submit: t.is_empty() || t == "submit", reset: t == "reset" }
        }
        _ => return None,
    })
}

/// True for controls that take typed text.
pub fn is_editable(dom: &Dom, id: NodeId) -> bool {
    matches!(control(dom, id), Some(Control::Text { .. } | Control::TextArea)) && !is_disabled(dom, id) && dom.attr(id, "readonly").is_none()
}

pub fn is_disabled(dom: &Dom, id: NodeId) -> bool {
    let mut cur = Some(id);
    while let Some(n) = cur {
        if matches!(dom.tag(n), Some("input" | "button" | "select" | "textarea" | "fieldset")) && dom.attr(n, "disabled").is_some() {
            return true;
        }
        cur = dom.nodes[n].parent;
    }
    false
}

/// The current value of a text control.
pub fn value(dom: &Dom, id: NodeId) -> String {
    match dom.tag(id) {
        Some("textarea") => {
            let t = dom.text_content(id);
            t.strip_prefix('\n').map(String::from).unwrap_or(t)
        }
        Some("select") => selected_option(dom, id).map(|o| option_value(dom, o)).unwrap_or_default(),
        _ => dom.attr(id, "value").unwrap_or("").to_string(),
    }
}

pub fn set_value(dom: &mut Dom, id: NodeId, v: &str) {
    dom.form_state.dirty.insert(id);
    if dom.tag(id) == Some("textarea") {
        // Edit the existing text node when there is one, so no new nodes need styling.
        let kids = dom.nodes[id].children.clone();
        if let [only] = kids[..] {
            if let NodeData::Text(t) = &mut dom.nodes[only].data {
                *t = v.to_string();
                return;
            }
        }
        dom.remove_children(id);
        dom.add(NodeData::Text(v.to_string()), id);
    } else {
        dom.set_attr(id, "value", v);
    }
}

pub fn options(dom: &Dom, select: NodeId) -> Vec<NodeId> {
    dom.descendants(select).into_iter().filter(|&n| dom.tag(n) == Some("option")).collect()
}

pub fn option_label(dom: &Dom, opt: NodeId) -> String {
    dom.attr(opt, "label").map(String::from).unwrap_or_else(|| dom.text_content(opt).split_whitespace().collect::<Vec<_>>().join(" "))
}

pub fn option_value(dom: &Dom, opt: NodeId) -> String {
    dom.attr(opt, "value").map(String::from).unwrap_or_else(|| option_label(dom, opt))
}

/// The selected <option>: the last one marked `selected`, else the first enabled one.
pub fn selected_option(dom: &Dom, select: NodeId) -> Option<NodeId> {
    let opts = options(dom, select);
    opts.iter().rev().find(|&&o| dom.attr(o, "selected").is_some()).copied().or_else(|| opts.first().copied())
}

pub fn select_option(dom: &mut Dom, select: NodeId, opt: NodeId) {
    for o in options(dom, select) {
        if o == opt {
            dom.set_attr(o, "selected", "");
        } else {
            dom.remove_attr(o, "selected");
        }
    }
}

/// The form an element belongs to: its `form` attribute, else the nearest <form> ancestor.
pub fn form_of(dom: &Dom, id: NodeId) -> Option<NodeId> {
    if let Some(fid) = dom.attr(id, "form") {
        return dom.descendants(Dom::ROOT).into_iter().find(|&n| dom.tag(n) == Some("form") && dom.attr(n, "id") == Some(fid));
    }
    let mut cur = dom.nodes[id].parent;
    while let Some(n) = cur {
        if dom.tag(n) == Some("form") {
            return Some(n);
        }
        cur = dom.nodes[n].parent;
    }
    None
}

/// Every control that belongs to `form`, in document order.
pub fn controls(dom: &Dom, form: NodeId) -> Vec<NodeId> {
    dom.descendants(Dom::ROOT)
        .into_iter()
        .filter(|&n| matches!(dom.tag(n), Some("input" | "select" | "textarea" | "button")) && form_of(dom, n) == Some(form))
        .collect()
}

/// Check a box or pick a radio button (unchecking the others in its group).
pub fn toggle(dom: &mut Dom, id: NodeId) {
    match control(dom, id) {
        Some(Control::Checkbox) => {
            if dom.attr(id, "checked").is_some() {
                dom.remove_attr(id, "checked");
            } else {
                dom.set_attr(id, "checked", "");
            }
        }
        Some(Control::Radio) => {
            if let Some(name) = dom.attr(id, "name").map(String::from) {
                let form = form_of(dom, id);
                let group: Vec<NodeId> = dom
                    .descendants(Dom::ROOT)
                    .into_iter()
                    .filter(|&n| n != id && dom.tag(n) == Some("input") && dom.attr(n, "name") == Some(&name) && form_of(dom, n) == form)
                    .collect();
                for n in group {
                    dom.remove_attr(n, "checked");
                }
            }
            dom.set_attr(id, "checked", "");
        }
        _ => {}
    }
}

/// The control a <label> points at: its `for` target, else the first control inside it.
pub fn label_target(dom: &Dom, label: NodeId) -> Option<NodeId> {
    if let Some(f) = dom.attr(label, "for") {
        return dom.descendants(Dom::ROOT).into_iter().find(|&n| dom.attr(n, "id") == Some(f) && control(dom, n).is_some());
    }
    dom.descendants(label).into_iter().find(|&n| n != label && control(dom, n).is_some())
}

/// Name/value pairs a form submits. `submitter` is the button that was pressed, if any.
pub fn form_data(dom: &Dom, form: NodeId, submitter: Option<NodeId>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for n in controls(dom, form) {
        if is_disabled(dom, n) {
            continue;
        }
        let Some(name) = dom.attr(n, "name").filter(|s| !s.is_empty()) else { continue };
        match control(dom, n) {
            Some(Control::Checkbox | Control::Radio) => {
                if dom.attr(n, "checked").is_some() {
                    out.push((name.to_string(), dom.attr(n, "value").unwrap_or("on").to_string()));
                }
            }
            Some(Control::Button { .. }) => {
                if Some(n) == submitter {
                    out.push((name.to_string(), dom.attr(n, "value").unwrap_or("").to_string()));
                }
            }
            Some(Control::Select) => {
                if dom.attr(n, "multiple").is_some() {
                    for o in options(dom, n).into_iter().filter(|&o| dom.attr(o, "selected").is_some()) {
                        out.push((name.to_string(), option_value(dom, o)));
                    }
                } else if let Some(o) = selected_option(dom, n) {
                    out.push((name.to_string(), option_value(dom, o)));
                }
            }
            // A file picker sends its files' names here; `submission` sends the files themselves.
            Some(Control::Other) if is_file(dom, n) => match dom.form_state.files.get(&n).filter(|f| !f.is_empty()) {
                Some(files) => out.extend(files.iter().map(|f| (name.to_string(), f.name.clone()))),
                None => out.push((name.to_string(), String::new())),
            },
            Some(_) => out.push((name.to_string(), value(dom, n).replace("\r\n", "\n").replace('\n', "\r\n"))),
            None if dom.tag(n) == Some("input") => out.push((name.to_string(), dom.attr(n, "value").unwrap_or("").to_string())), // hidden
            None => {}
        }
    }
    out
}

/// application/x-www-form-urlencoded
pub fn urlencode(pairs: &[(String, String)]) -> String {
    fn enc(s: &str, out: &mut String) {
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
    }
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        enc(k, &mut out);
        out.push('=');
        enc(v, &mut out);
    }
    out
}

pub fn is_file(dom: &Dom, id: NodeId) -> bool {
    dom.tag(id) == Some("input") && dom.attr(id, "type").is_some_and(|t| t.trim().eq_ignore_ascii_case("file"))
}

/// Where and how a form submits.
pub struct Submission {
    pub url: String,
    /// `Some(body)` for POST.
    pub post: Option<PostBody>,
}

/// A POST request's body and its Content-Type.
#[derive(Debug, Clone, PartialEq)]
pub struct PostBody {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl PostBody {
    pub fn urlencoded(data: String) -> PostBody {
        PostBody { content_type: "application/x-www-form-urlencoded".into(), bytes: data.into_bytes() }
    }

    /// The body as a query string (for forms "posted" to local files, which can only be read).
    pub fn query(&self) -> String {
        if self.content_type.starts_with("application/x-www-form-urlencoded") { String::from_utf8_lossy(&self.bytes).into_owned() } else { String::new() }
    }
}

/// multipart/form-data: every field as its own part, picked files with their contents.
pub fn multipart(dom: &Dom, form: NodeId, submitter: Option<NodeId>, boundary: &str) -> Vec<u8> {
    let esc = |s: &str| s.replace('"', "%22").replace('\r', "%0D").replace('\n', "%0A");
    let mut out: Vec<u8> = Vec::new();
    let mut files_done = std::collections::HashSet::new();
    for (name, value) in form_data(dom, form, submitter) {
        // Pair each file input's entries back up with its files, in order.
        let file_input = controls(dom, form).into_iter().find(|&n| is_file(dom, n) && dom.attr(n, "name") == Some(name.as_str()) && !files_done.contains(&n) && !is_disabled(dom, n));
        out.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"", esc(&name)).as_bytes());
        match file_input {
            Some(n) => {
                let files = dom.form_state.files.get(&n).cloned().unwrap_or_default();
                let i = out_count(&files, &value);
                let file = files.get(i);
                if i + 1 >= files.len() {
                    files_done.insert(n);
                }
                let (fname, mime) = file.map_or((String::new(), "application/octet-stream".to_string()), |f| (f.name.clone(), f.mime.clone()));
                out.extend_from_slice(format!("; filename=\"{}\"\r\nContent-Type: {mime}\r\n\r\n", esc(&fname)).as_bytes());
                if let Some(f) = file {
                    out.extend(std::fs::read(&f.path).unwrap_or_default());
                }
            }
            None => {
                out.extend_from_slice(b"\r\n\r\n");
                out.extend_from_slice(value.as_bytes());
            }
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

/// Which of an input's files a form entry named `value` is (entries come in file order).
fn out_count(files: &[crate::validation::ChosenFile], value: &str) -> usize {
    files.iter().position(|f| f.name == value).unwrap_or(0)
}

pub fn submission(dom: &Dom, form: NodeId, submitter: Option<NodeId>, page_url: &str) -> Submission {
    let pick = |attr: &str| submitter.and_then(|s| dom.attr(s, &format!("form{attr}"))).or_else(|| dom.attr(form, attr));
    let action = pick("action").filter(|a| !a.trim().is_empty()).map(|a| crate::net::resolve(page_url, a.trim())).unwrap_or_else(|| page_url.to_string());
    let method = pick("method").unwrap_or("get").to_ascii_lowercase();
    if method == "post" {
        let enctype = pick("enctype").unwrap_or("").trim().to_ascii_lowercase();
        let body = match enctype.as_str() {
            "multipart/form-data" => {
                let boundary = format!("----IronswiftFormBoundary{:016x}", boundary_seed());
                PostBody { bytes: multipart(dom, form, submitter, &boundary), content_type: format!("multipart/form-data; boundary={boundary}") }
            }
            "text/plain" => {
                let text: String = form_data(dom, form, submitter).into_iter().map(|(k, v)| format!("{k}={v}\r\n")).collect();
                PostBody { content_type: "text/plain".into(), bytes: text.into_bytes() }
            }
            _ => PostBody::urlencoded(urlencode(&form_data(dom, form, submitter))),
        };
        Submission { url: action, post: Some(body) }
    } else {
        let data = urlencode(&form_data(dom, form, submitter));
        let base = action.split('#').next().unwrap_or("").split('?').next().unwrap_or("").to_string();
        Submission { url: format!("{base}?{data}"), post: None }
    }
}

/// A boundary that won't turn up inside an uploaded file by chance.
fn boundary_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish()
}

/// The form's default button (its first submit button), for pressing Enter in a field.
pub fn default_button(dom: &Dom, form: NodeId) -> Option<NodeId> {
    controls(dom, form).into_iter().find(|&n| matches!(control(dom, n), Some(Control::Button { submit: true, .. })))
}

/// Put every control in the form back to its value from the page's HTML.
/// (We keep live values in attributes, so this restores what was saved at load.)
pub fn reset(dom: &mut Dom, form: NodeId, defaults: &[(NodeId, Option<String>, bool)]) {
    let ctrls = controls(dom, form);
    for &(n, ref v, on) in defaults {
        if dom.tag(n) == Some("option") {
            let select = dom.nodes[n].parent.and_then(|p| if dom.tag(p) == Some("select") { Some(p) } else { dom.nodes[p].parent });
            if select.is_some_and(|s| ctrls.contains(&s)) {
                if on {
                    dom.set_attr(n, "selected", "");
                } else {
                    dom.remove_attr(n, "selected");
                }
            }
            continue;
        }
        let checked = on;
        if !ctrls.contains(&n) {
            continue;
        }
        match v {
            Some(v) => set_value(dom, n, v),
            None if dom.tag(n) != Some("textarea") => dom.remove_attr(n, "value"),
            None => {}
        }
        if checked {
            dom.set_attr(n, "checked", "");
        } else {
            dom.remove_attr(n, "checked");
        }
    }
    for n in ctrls {
        dom.form_state.dirty.remove(&n);
        dom.form_state.files.remove(&n);
    }
}

/// Snapshot of every control's starting value, for form.reset().
pub fn defaults(dom: &Dom) -> Vec<(NodeId, Option<String>, bool)> {
    dom.descendants(Dom::ROOT)
        .into_iter()
        .filter(|&n| matches!(dom.tag(n), Some("input" | "textarea" | "option")))
        .map(|n| match dom.tag(n) {
            Some("option") => (n, None, dom.attr(n, "selected").is_some()),
            Some("textarea") => (n, Some(value(dom, n)), false),
            _ => (n, dom.attr(n, "value").map(String::from), dom.attr(n, "checked").is_some()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn get_and_post() {
        let dom = crate::html::parse(
            r#"<form action="/search" method="get">
                 <input name="q" value="rust browser">
                 <input type="checkbox" name="safe" checked>
                 <input type="checkbox" name="off">
                 <input type="radio" name="s" value="a"><input type="radio" name="s" value="b" checked>
                 <select name="lang"><option>en</option><option value="fr" selected>French</option></select>
                 <textarea name="t">line1
line2</textarea>
                 <input type="hidden" name="h" value="1&2">
                 <input type="submit" name="go" value="Go">
                 <input name="dis" disabled value="x">
               </form>"#,
        );
        let form = dom.find_all("form")[0];
        let s = submission(&dom, form, None, "https://example.com/a/b?x=1");
        assert_eq!(s.url, "https://example.com/search?q=rust+browser&safe=on&s=b&lang=fr&t=line1%0D%0Aline2&h=1%262");
        assert!(s.post.is_none());
    }

    #[test]
    fn multipart_upload() {
        let dir = std::env::temp_dir().join(format!("ironswift-mp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");
        std::fs::write(&path, b"hello\x00bytes").unwrap();
        let mut dom = crate::html::parse(r#"<form method=post enctype="multipart/form-data"><input name="t" value="a&quot;b"><input type=file name="f"><input type=file name="g"></form>"#);
        let inputs = dom.find_all("input");
        dom.form_state.files.insert(inputs[1], vec![crate::validation::ChosenFile::from_path(&path).unwrap()]);
        let form = dom.find_all("form")[0];
        let body = multipart(&dom, form, None, "XX");
        let text = String::from_utf8_lossy(&body);
        assert_eq!(text, "--XX\r\nContent-Disposition: form-data; name=\"t\"\r\n\r\na\"b\r\n--XX\r\nContent-Disposition: form-data; name=\"f\"; filename=\"note.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\0bytes\r\n--XX\r\nContent-Disposition: form-data; name=\"g\"; filename=\"\"\r\nContent-Type: application/octet-stream\r\n\r\n\r\n--XX--\r\n");
        let s = submission(&dom, form, None, "https://x.org/");
        assert!(s.post.unwrap().content_type.starts_with("multipart/form-data; boundary=----IronswiftFormBoundary"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
