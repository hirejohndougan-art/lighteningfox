//! Autofill and saved passwords.
//!
//! - **Logins**: when a form with a password is sent, the browser offers to save the user name
//!   and password for that site. On a later visit, clicking the user name or password field
//!   offers the saved logins; picking one fills both fields.
//! - **Form entries**: values typed into fields for names, email addresses, phone numbers and
//!   addresses (by their `autocomplete` token, else their name) are remembered per kind and
//!   offered in fields of the same kind on any site. `autocomplete=off` opts a field out.
//!
//! Everything lives in the profile folder, readable only by the user (0600). Passwords are
//! stored scrambled with a key kept in a separate file, so they never sit in plain text on
//! disk; anyone who can read both files as this user can still recover them (the same as
//! Firefox without a primary password).

use std::path::PathBuf;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::dom::{Dom, NodeId};
use crate::forms::{self, Control};

#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    /// scheme://host[:port] of the page the form was on.
    pub origin: String,
    pub username: String,
    pub password: String,
    /// Seconds since 1970, of the last save or use.
    pub used: u64,
}

#[derive(Default)]
struct Store {
    loaded: bool,
    logins: Vec<Login>,
    /// Sites the user said never to save for.
    never: Vec<String>,
    /// (kind, value), most recent last.
    entries: Vec<(String, String)>,
}

static STORE: Mutex<Store> = Mutex::new(Store { loaded: false, logins: Vec::new(), never: Vec::new(), entries: Vec::new() });

fn dir() -> Option<PathBuf> {
    crate::cookies::profile_dir()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// scheme://host[:port]
pub fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    Some(u.origin().ascii_serialization())
}

// ---------------------------------------------------------------- storage

fn write_private(path: &std::path::Path, data: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, data).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, path);
    }
}

/// The key that scrambles stored passwords (made on first use).
fn key() -> Option<[u8; 32]> {
    let path = dir()?.join("logins.key");
    if let Ok(b) = std::fs::read(&path) {
        if b.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&b);
            return Some(k);
        }
    }
    let mut k = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut k)).ok()?;
    write_private(&path, &k);
    Some(k)
}

fn keystream(key: &[u8; 32], nonce: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n + 32);
    let mut i = 0u32;
    while out.len() < n {
        let mut h = Sha256::new();
        h.update(key);
        h.update(nonce);
        h.update(i.to_le_bytes());
        out.extend_from_slice(&h.finalize());
        i += 1;
    }
    out.truncate(n);
    out
}

fn seal(key: &[u8; 32], plain: &str) -> String {
    use base64::Engine;
    let mut nonce = [0u8; 12];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut nonce));
    let ks = keystream(key, &nonce, plain.len());
    let mut out = nonce.to_vec();
    out.extend(plain.bytes().zip(ks).map(|(a, b)| a ^ b));
    base64::engine::general_purpose::STANDARD.encode(out)
}

fn open_sealed(key: &[u8; 32], sealed: &str) -> Option<String> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(sealed).ok()?;
    if raw.len() < 12 {
        return None;
    }
    let (nonce, body) = raw.split_at(12);
    let ks = keystream(key, nonce, body.len());
    String::from_utf8(body.iter().zip(ks).map(|(a, b)| a ^ b).collect()).ok()
}

/// Fields are tab-separated; tabs, newlines and backslashes inside them are escaped.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn load(s: &mut Store) {
    if s.loaded {
        return;
    }
    s.loaded = true;
    let Some(d) = dir() else { return };
    if let (Ok(text), Some(k)) = (std::fs::read_to_string(d.join("logins.tsv")), key()) {
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["login", origin, user, pass, used] => {
                    if let Some(password) = open_sealed(&k, pass) {
                        s.logins.push(Login { origin: unesc(origin), username: unesc(user), password, used: used.parse().unwrap_or(0) });
                    }
                }
                ["never", origin] => s.never.push(unesc(origin)),
                _ => {}
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(d.join("formhistory.tsv")) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('\t') {
                s.entries.push((unesc(k), unesc(v)));
            }
        }
    }
}

fn save(s: &Store) {
    let (Some(d), Some(k)) = (dir(), key()) else { return };
    let mut out = String::new();
    for l in &s.logins {
        out.push_str(&format!("login\t{}\t{}\t{}\t{}\n", esc(&l.origin), esc(&l.username), seal(&k, &l.password), l.used));
    }
    for n in &s.never {
        out.push_str(&format!("never\t{}\n", esc(n)));
    }
    write_private(&d.join("logins.tsv"), out.as_bytes());
    let hist: String = s.entries.iter().map(|(k, v)| format!("{}\t{}\n", esc(k), esc(v))).collect();
    write_private(&d.join("formhistory.tsv"), hist.as_bytes());
}

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    load(&mut s);
    f(&mut s)
}

pub fn logins_for(origin: &str) -> Vec<Login> {
    let mut v: Vec<Login> = with_store(|s| s.logins.iter().filter(|l| l.origin == origin).cloned().collect());
    v.sort_by(|a, b| b.used.cmp(&a.used));
    v
}

/// Save (or update) a login.
pub fn save_login(login: Login) {
    with_store(|s| {
        s.logins.retain(|l| !(l.origin == login.origin && l.username == login.username));
        s.logins.push(login);
        save(s);
    })
}

pub fn never_for(origin: &str) {
    with_store(|s| {
        if !s.never.iter().any(|n| n == origin) {
            s.never.push(origin.to_string());
            save(s);
        }
    })
}

pub fn mark_used(origin: &str, username: &str) {
    with_store(|s| {
        if let Some(l) = s.logins.iter_mut().find(|l| l.origin == origin && l.username == username) {
            l.used = now();
            save(s);
        }
    })
}

/// Forget everything (Clear browsing data).
#[allow(dead_code)]
pub fn clear_form_history() {
    with_store(|s| {
        s.entries.clear();
        save(s);
    })
}

// ---------------------------------------------------------------- reading forms

/// What kind of information a field holds, for form history ("email", "tel", "name"…).
pub fn field_kind(dom: &Dom, id: NodeId) -> Option<String> {
    if !matches!(forms::control(dom, id), Some(Control::Text { password: false })) || dom.tag(id) != Some("input") {
        return None;
    }
    let ty = dom.attr(id, "type").unwrap_or("text").trim().to_ascii_lowercase();
    if !matches!(ty.as_str(), "text" | "email" | "tel" | "url" | "search" | "") {
        return None;
    }
    let auto = dom.attr(id, "autocomplete").unwrap_or("").trim().to_ascii_lowercase();
    if auto == "off" || auto.contains("one-time-code") || auto.contains("cc-") {
        return None;
    }
    const KNOWN: &[&str] = &["name", "given-name", "family-name", "email", "tel", "street-address", "address-line1", "address-line2", "address-level2", "address-level1", "postal-code", "country-name", "organization", "username"];
    // The last token of autocomplete ("shipping email" → "email").
    if let Some(tok) = auto.split_whitespace().last().filter(|t| KNOWN.contains(t)) {
        return Some(tok.to_string());
    }
    if ty == "email" || ty == "tel" {
        return Some(ty);
    }
    // Guess from the field's name or id.
    let name = format!("{} {}", dom.attr(id, "name").unwrap_or(""), dom.attr(id, "id").unwrap_or("")).to_ascii_lowercase();
    let has = |w: &[&str]| w.iter().any(|x| name.contains(x));
    Some(
        if has(&["email", "e-mail"]) {
            "email"
        } else if has(&["phone", "tel"]) {
            "tel"
        } else if has(&["zip", "postal", "postcode"]) {
            "postal-code"
        } else if has(&["first"]) && has(&["name"]) {
            "given-name"
        } else if has(&["last", "surname"]) {
            "family-name"
        } else if has(&["city", "town"]) {
            "address-level2"
        } else if has(&["state", "province", "region"]) {
            "address-level1"
        } else if has(&["address", "street"]) {
            "street-address"
        } else if has(&["company", "organization", "organisation"]) {
            "organization"
        } else if has(&["user", "login"]) {
            "username"
        } else if has(&["name"]) {
            "name"
        } else {
            return None;
        },
    )
    .map(String::from)
}

/// The form's login fields: (user name field, password field).
pub fn login_fields(dom: &Dom, form: NodeId) -> Option<(Option<NodeId>, NodeId)> {
    let ctrls = forms::controls(dom, form);
    let pass = ctrls.iter().copied().find(|&n| matches!(forms::control(dom, n), Some(Control::Text { password: true })))?;
    // The text field closest before the password.
    let pos = ctrls.iter().position(|&n| n == pass)?;
    let user = ctrls[..pos].iter().rev().copied().find(|&n| matches!(forms::control(dom, n), Some(Control::Text { password: false })) && dom.tag(n) == Some("input"));
    Some((user, pass))
}

/// A login waiting for the user to say whether to save it.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub login: Login,
    /// It changes the password of a login already saved.
    pub update: bool,
}

static OFFER: Mutex<Option<Offer>> = Mutex::new(None);

/// Picked up by the window loop after a form is sent.
pub fn take_offer() -> Option<Offer> {
    OFFER.lock().ok()?.take()
}

/// A form is being sent from `page_url`: remember its typed entries, and offer to save its
/// login. Nothing is kept from private windows.
pub fn form_submitted(dom: &Dom, form: NodeId, page_url: &str, private: bool) {
    if private {
        return;
    }
    let Some(origin) = origin_of(page_url) else { return };
    let mut new_entries = Vec::new();
    for n in forms::controls(dom, form) {
        if let Some(kind) = field_kind(dom, n) {
            let v = forms::value(dom, n).trim().to_string();
            if !v.is_empty() && v.chars().count() <= 200 && dom.form_state.dirty.contains(&n) {
                new_entries.push((kind, v));
            }
        }
    }
    if !new_entries.is_empty() {
        with_store(|s| {
            for e in new_entries {
                s.entries.retain(|x| *x != e);
                s.entries.push(e);
            }
            // Keep the most recent 500.
            let extra = s.entries.len().saturating_sub(500);
            s.entries.drain(..extra);
            save(s);
        });
    }
    let Some((user, pass)) = login_fields(dom, form) else { return };
    let password = forms::value(dom, pass);
    if password.is_empty() || with_store(|s| s.never.contains(&origin)) {
        return;
    }
    let username = user.map(|u| forms::value(dom, u)).unwrap_or_default();
    let existing = logins_for(&origin).into_iter().find(|l| l.username == username);
    if existing.as_ref().is_some_and(|l| l.password == password) {
        mark_used(&origin, &username);
        return;
    }
    let offer = Offer { update: existing.is_some(), login: Login { origin, username, password, used: now() } };
    if let Ok(mut o) = OFFER.lock() {
        *o = Some(offer);
    }
}

/// One suggestion under a field: what it shows and what it fills in.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub fill: Vec<(NodeId, String)>,
    /// A saved login (origin, user name), so using it can be remembered.
    pub login: Option<(String, String)>,
}

/// Suggestions for a field that was just clicked.
pub fn suggestions(dom: &Dom, field: NodeId, page_url: &str) -> Vec<Suggestion> {
    let mut out = Vec::new();
    // Logins, for the user name and password fields of a login form.
    if let (Some(form), Some(origin)) = (forms::form_of(dom, field), origin_of(page_url)) {
        if let Some((user, pass)) = login_fields(dom, form) {
            if field == pass || Some(field) == user {
                for l in logins_for(&origin) {
                    let mut fill = vec![(pass, l.password.clone())];
                    if let Some(u) = user {
                        fill.push((u, l.username.clone()));
                    }
                    let label = if l.username.is_empty() { "(no user name)".into() } else { l.username.clone() };
                    out.push(Suggestion { label, detail: "Saved login".into(), fill, login: Some((l.origin.clone(), l.username.clone())) });
                }
                if !out.is_empty() {
                    return out;
                }
            }
        }
    }
    // Form history for this kind of field, matching what's typed so far.
    if let Some(kind) = field_kind(dom, field) {
        let typed = forms::value(dom, field).to_lowercase();
        let values: Vec<String> = with_store(|s| s.entries.iter().rev().filter(|(k, _)| *k == kind).map(|(_, v)| v.clone()).collect());
        for v in values.into_iter().filter(|v| v.to_lowercase().starts_with(&typed) && v.to_lowercase() != typed).take(6) {
            out.push(Suggestion { label: v.clone(), detail: String::new(), fill: vec![(field, v)], login: None });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealing_round_trip() {
        let k = [7u8; 32];
        let s = seal(&k, "hunter2 ünïcode");
        assert!(!s.contains("hunter2"));
        assert_eq!(open_sealed(&k, &s).as_deref(), Some("hunter2 ünïcode"));
        assert_eq!(unesc(&esc("a\tb\\c\nd")), "a\tb\\c\nd");
    }

    #[test]
    fn kinds_and_login_fields() {
        let dom = crate::html::parse(r#"<form><input name="email"><input id="fname" name="first_name"><input autocomplete="shipping postal-code" name="x"><input name="q" autocomplete="off"><input name="user"><input type=password name=pw></form>"#);
        let i = dom.find_all("input");
        assert_eq!(field_kind(&dom, i[0]).as_deref(), Some("email"));
        assert_eq!(field_kind(&dom, i[1]).as_deref(), Some("given-name"));
        assert_eq!(field_kind(&dom, i[2]).as_deref(), Some("postal-code"));
        assert_eq!(field_kind(&dom, i[3]), None);
        let form = dom.find_all("form")[0];
        assert_eq!(login_fields(&dom, form), Some((Some(i[4]), i[5])));
    }
}
