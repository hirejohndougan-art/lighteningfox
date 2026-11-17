//! Form validation (the HTML constraint validation API): `required`, `pattern`,
//! `min`/`max`/`step`, `minlength`/`maxlength`, `type=email`/`url`, badly typed numbers and
//! messages set by scripts with `setCustomValidity`. The browser checks a form before
//! submitting it and shows the first problem in a bubble (`formui.rs`); scripts see the same
//! results through `validity`, `checkValidity()` and `reportValidity()` (`forms.js`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::dom::{Dom, NodeId};
use crate::forms::{self, Control};

/// Per-document form state that isn't kept in attributes.
#[derive(Debug, Clone, Default)]
pub struct FormState {
    /// Messages from setCustomValidity (an empty message clears it).
    pub custom: HashMap<NodeId, String>,
    /// Controls whose value the user has typed into (minlength/maxlength only apply to those).
    pub dirty: HashSet<NodeId>,
    /// Files picked in `<input type=file>`.
    pub files: HashMap<NodeId, Vec<ChosenFile>>,
}

/// A file picked for upload.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenFile {
    pub path: std::path::PathBuf,
    pub name: String,
    pub size: u64,
    pub mime: String,
    /// Milliseconds since 1970.
    pub modified: f64,
}

impl ChosenFile {
    pub fn from_path(path: &std::path::Path) -> Option<ChosenFile> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_millis() as f64);
        Some(ChosenFile { path: path.to_path_buf(), mime: mime_for(&name).to_string(), name, size: meta.len(), modified })
    }
}

/// A file's type from its extension (what browsers send as the part's Content-Type).
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" | "text" | "log" => "text/plain",
        "htm" | "html" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        _ => "application/octet-stream",
    }
}

/// Which constraints a control fails (the ValidityState flags).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Validity {
    pub value_missing: bool,
    pub type_mismatch: bool,
    pub pattern_mismatch: bool,
    pub too_long: bool,
    pub too_short: bool,
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
    pub bad_input: bool,
    pub custom_error: bool,
}

impl Validity {
    pub fn valid(&self) -> bool {
        *self == Validity::default()
    }
}

fn input_type(dom: &Dom, id: NodeId) -> String {
    dom.attr(id, "type").unwrap_or("text").trim().to_ascii_lowercase()
}

/// A control the browser validates ("candidate for constraint validation").
pub fn will_validate(dom: &Dom, id: NodeId) -> bool {
    let Some(tag) = dom.tag(id) else { return false };
    if !matches!(tag, "input" | "select" | "textarea" | "button") || forms::is_disabled(dom, id) {
        return false;
    }
    // Inside a <datalist>: never.
    let mut cur = dom.nodes[id].parent;
    while let Some(p) = cur {
        if dom.tag(p) == Some("datalist") {
            return false;
        }
        cur = dom.nodes[p].parent;
    }
    match tag {
        "input" => {
            let t = input_type(dom, id);
            !matches!(t.as_str(), "hidden" | "reset" | "button" | "image") && dom.attr(id, "readonly").is_none()
        }
        "textarea" => dom.attr(id, "readonly").is_none(),
        "button" => matches!(forms::control(dom, id), Some(Control::Button { submit: true, .. })),
        _ => true,
    }
}

/// The value scripts and submission see for a single-line input (newlines stripped).
fn input_value(dom: &Dom, id: NodeId) -> String {
    forms::value(dom, id).replace(['\r', '\n'], "")
}

/// Check a control's constraints. The flags are worked out even for controls the browser
/// doesn't validate (disabled, read-only); `invalid_controls` and scripts' checkValidity()
/// skip those.
pub fn validity(dom: &Dom, state: &FormState, id: NodeId) -> Validity {
    let mut v = Validity::default();
    if !matches!(dom.tag(id), Some("input" | "select" | "textarea" | "button" | "fieldset" | "output" | "object")) {
        return v;
    }
    v.custom_error = state.custom.get(&id).is_some_and(|m| !m.is_empty());
    // "Suffering from being missing" needs a control the user could fill in.
    // (readonly doesn't apply to checkboxes, radio buttons and file pickers.)
    let readonly_applies = match dom.tag(id) {
        Some("textarea") => true,
        Some("input") => !matches!(input_type(dom, id).as_str(), "checkbox" | "radio" | "file"),
        _ => false,
    };
    let mutable = !forms::is_disabled(dom, id) && !(readonly_applies && dom.attr(id, "readonly").is_some());
    let required = dom.attr(id, "required").is_some() && mutable;
    match dom.tag(id) {
        Some("select") => {
            if required && dom.attr(id, "multiple").is_none() {
                // The first option, with an empty value, is a placeholder ("Choose one…").
                let opts = forms::options(dom, id);
                let sel = opts.iter().rev().find(|&&o| dom.attr(o, "selected").is_some()).copied().or_else(|| opts.first().copied());
                v.value_missing = match sel {
                    None => true,
                    Some(o) => forms::option_value(dom, o).is_empty() && Some(o) == opts.first().copied() && dom.nodes[o].parent == Some(id),
                };
            } else if required {
                v.value_missing = !forms::options(dom, id).iter().any(|&o| dom.attr(o, "selected").is_some());
            }
        }
        Some("textarea") => {
            let value = forms::value(dom, id);
            v.value_missing = required && value.is_empty();
            length_checks(dom, state, id, &value, &mut v);
        }
        Some("input") => input_checks(dom, state, id, required, &mut v),
        _ => {}
    }
    v
}

fn length_checks(dom: &Dom, state: &FormState, id: NodeId, value: &str, v: &mut Validity) {
    if value.is_empty() || !state.dirty.contains(&id) {
        return;
    }
    let len = value.encode_utf16().count();
    let n = |a: &str| dom.attr(id, a).and_then(|s| s.trim().parse::<usize>().ok());
    if n("maxlength").is_some_and(|m| len > m) {
        v.too_long = true;
    }
    if n("minlength").is_some_and(|m| len < m) {
        v.too_short = true;
    }
}

fn input_checks(dom: &Dom, state: &FormState, id: NodeId, required: bool, v: &mut Validity) {
    let t = input_type(dom, id);
    match t.as_str() {
        "checkbox" => {
            v.value_missing = required && dom.attr(id, "checked").is_none();
            return;
        }
        "radio" => {
            // The group needs one checked button if any of them is required.
            let name = dom.attr(id, "name").unwrap_or("");
            let group: Vec<NodeId> = if name.is_empty() {
                vec![id]
            } else {
                let form = forms::form_of(dom, id);
                dom.find_all("input")
                    .into_iter()
                    .filter(|&n| input_type(dom, n) == "radio" && dom.attr(n, "name") == Some(name) && forms::form_of(dom, n) == form)
                    .collect()
            };
            let any_required = group.iter().any(|&n| dom.attr(n, "required").is_some() && !forms::is_disabled(dom, n));
            v.value_missing = !forms::is_disabled(dom, id) && any_required && !group.iter().any(|&n| dom.attr(n, "checked").is_some());
            return;
        }
        "file" => {
            v.value_missing = required && state.files.get(&id).is_none_or(|f| f.is_empty());
            return;
        }
        "submit" | "color" => return,
        _ => {}
    }
    let mut value = input_value(dom, id);
    // A date or number the page set that doesn't parse reads as empty (value sanitization);
    // one the user typed is "bad input".
    let unparsable = Numeric::of(&t).is_some_and(|k| !value.is_empty() && k.parse(&value).is_none());
    if unparsable && !state.dirty.contains(&id) {
        value.clear();
    }
    if t == "range" {
        return; // always sanitized to a valid number
    }
    v.value_missing = required && value.is_empty();
    if value.is_empty() {
        return;
    }
    match t.as_str() {
        "email" => {
            let multiple = dom.attr(id, "multiple").is_some();
            let parts: Vec<&str> = if multiple { value.split(',').map(str::trim).collect() } else { vec![value.trim()] };
            v.type_mismatch = parts.iter().any(|p| !valid_email(p));
            pattern_check(dom, id, &parts, v);
        }
        "url" => {
            v.type_mismatch = url::Url::parse(value.trim()).is_err();
            pattern_check(dom, id, &[value.as_str()], v);
        }
        "text" | "search" | "tel" | "password" => pattern_check(dom, id, &[value.as_str()], v),
        _ => {}
    }
    if matches!(t.as_str(), "text" | "search" | "url" | "tel" | "email" | "password") {
        length_checks(dom, state, id, &value, v);
    }
    if let Some(kind) = Numeric::of(&t) {
        match kind.parse(&value) {
            None => v.bad_input = true,
            Some(x) => {
                let attr = |a: &str| dom.attr(id, a).and_then(|s| kind.parse(s.trim()));
                match (attr("min"), attr("max")) {
                    // A time range can wrap past midnight (min 22:00, max 06:00).
                    (Some(min), Some(max)) if kind == Numeric::Time && min > max => {
                        if x < min && x > max {
                            v.range_underflow = true;
                            v.range_overflow = true;
                        }
                    }
                    (min, max) => {
                        v.range_underflow = min.is_some_and(|m| x < m);
                        v.range_overflow = max.is_some_and(|m| x > m);
                    }
                }
                if let Some(step) = kind.step(dom.attr(id, "step")) {
                    let base = attr("min").unwrap_or(kind.default_base());
                    let q = (x - base) / step;
                    v.step_mismatch = (q - q.round()).abs() > 1e-7 * q.abs().max(1.0);
                }
            }
        }
    }
}

fn pattern_check(dom: &Dom, id: NodeId, values: &[&str], v: &mut Validity) {
    let Some(pat) = dom.attr(id, "pattern") else { return };
    // The pattern must match the whole value, with the "v" flag's Unicode rules
    // (regress supports "u", which accepts the same patterns for ordinary use).
    // (Checked on its own first, so a pattern like "a)(b" can't escape the group.)
    if regress::Regex::with_flags(pat, "v").is_err() {
        return;
    }
    let Ok(re) = regress::Regex::with_flags(&format!("^(?:{pat})$"), "v") else { return };
    v.pattern_mismatch = values.iter().any(|s| !s.is_empty() && re.find(s).is_none());
}

/// The HTML standard's "valid email address" (a simplified RFC 5322).
pub fn valid_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    let local_ok = !local.is_empty() && local.chars().all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c));
    let label_ok = |l: &str| {
        !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-') && !l.ends_with('-')
    };
    local_ok && !domain.is_empty() && domain.split('.').all(label_ok)
}

/// Number-like input types, with values compared as numbers (dates in milliseconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Numeric {
    Number,
    Date,
    Month,
    Week,
    Time,
    DateTime,
}

impl Numeric {
    pub fn of(t: &str) -> Option<Numeric> {
        Some(match t {
            "number" => Numeric::Number,
            "date" => Numeric::Date,
            "month" => Numeric::Month,
            "week" => Numeric::Week,
            "time" => Numeric::Time,
            "datetime-local" => Numeric::DateTime,
            _ => return None,
        })
    }

    pub fn parse(self, s: &str) -> Option<f64> {
        match self {
            Numeric::Number => parse_number(s),
            Numeric::Date => parse_date(s).map(|(y, m, d)| days_from_civil(y, m, d) as f64 * DAY),
            Numeric::Month => {
                let (y, m) = parse_month(s)?;
                Some(((y - 1970) * 12 + (m as i64 - 1)) as f64)
            }
            Numeric::Week => {
                let (y, w) = parse_week(s)?;
                // Monday of week 1 is the Monday of the week containing January 4th.
                let jan4 = days_from_civil(y, 1, 4);
                let monday = jan4 - (jan4 + 3).rem_euclid(7);
                Some((monday + (w as i64 - 1) * 7) as f64 * DAY)
            }
            Numeric::Time => parse_time(s),
            Numeric::DateTime => {
                let (d, t) = s.split_once(['T', ' '])?;
                let (y, m, dd) = parse_date(d)?;
                Some(days_from_civil(y, m, dd) as f64 * DAY + parse_time(t)?)
            }
        }
    }

    /// The step in value units; None for `step=any`.
    pub fn step(self, attr: Option<&str>) -> Option<f64> {
        let (default, scale) = match self {
            Numeric::Number => (1.0, 1.0),
            Numeric::Date => (1.0, DAY),
            Numeric::Month => (1.0, 1.0),
            Numeric::Week => (1.0, 7.0 * DAY),
            Numeric::Time | Numeric::DateTime => (60.0, 1000.0),
        };
        match attr.map(str::trim) {
            Some(a) if a.eq_ignore_ascii_case("any") => None,
            Some(a) => Some(parse_number(a).filter(|&n| n > 0.0).unwrap_or(default) * scale),
            None => Some(default * scale),
        }
    }

    fn default_base(self) -> f64 {
        if self == Numeric::Week { -259_200_000.0 } else { 0.0 }
    }
}

const DAY: f64 = 86_400_000.0;

/// A "valid floating-point number" (no leading +, no trailing junk, no Infinity).
fn parse_number(s: &str) -> Option<f64> {
    let ok = !s.is_empty()
        && !s.starts_with('+')
        && s.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | '.' | 'e' | 'E' | '+'))
        && !s.ends_with('.');
    ok.then(|| s.parse::<f64>().ok()).flatten().filter(|n| n.is_finite())
}

fn digits(s: &str, n: usize) -> Option<u32> {
    (s.len() == n && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

pub fn parse_month(s: &str) -> Option<(i64, u32)> {
    let (y, m) = s.rsplit_once('-')?;
    let y: i64 = (y.len() >= 4 && y.bytes().all(|b| b.is_ascii_digit())).then(|| y.parse().ok()).flatten().filter(|&y| y > 0)?;
    let m = digits(m, 2).filter(|m| (1..=12).contains(m))?;
    Some((y, m))
}

pub fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let (ym, d) = s.rsplit_once('-')?;
    let (y, m) = parse_month(ym)?;
    let d = digits(d, 2).filter(|&d| d >= 1 && d <= days_in_month(y, m))?;
    Some((y, m, d))
}

fn parse_week(s: &str) -> Option<(i64, u32)> {
    let (y, w) = s.split_once("-W")?;
    let y: i64 = (y.len() >= 4 && y.bytes().all(|b| b.is_ascii_digit())).then(|| y.parse().ok()).flatten()?;
    let w = digits(w, 2)?;
    // Years starting on a Thursday (or leap years starting on a Wednesday) have 53 weeks.
    let jan1 = (days_from_civil(y, 1, 1) + 3).rem_euclid(7); // 0 = Monday
    let leap = days_in_month(y, 2) == 29;
    let max = if jan1 == 3 || (leap && jan1 == 2) { 53 } else { 52 };
    (1..=max).contains(&w).then_some((y, w))
}

/// "HH:MM", "HH:MM:SS" or "HH:MM:SS.mmm", in milliseconds.
pub fn parse_time(s: &str) -> Option<f64> {
    let mut it = s.splitn(3, ':');
    let h = digits(it.next()?, 2).filter(|&h| h < 24)?;
    let m = digits(it.next()?, 2).filter(|&m| m < 60)?;
    let sec = match it.next() {
        None => 0.0,
        Some(rest) => {
            let (whole, frac) = rest.split_once('.').map_or((rest, None), |(a, b)| (a, Some(b)));
            let s = digits(whole, 2).filter(|&s| s < 60)? as f64;
            match frac {
                None => s,
                Some(f) if !f.is_empty() && f.len() <= 3 && f.bytes().all(|b| b.is_ascii_digit()) => s + format!("0.{f}").parse::<f64>().ok()?,
                Some(_) => return None,
            }
        }
    };
    Some(((h * 60 + m) as f64 * 60.0 + sec) * 1000.0)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The (year, month, day) for days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// The message shown for a control's first problem (what `validationMessage` returns).
pub fn message(dom: &Dom, state: &FormState, id: NodeId, v: &Validity) -> String {
    if v.custom_error {
        return state.custom.get(&id).cloned().unwrap_or_default();
    }
    let t = if dom.tag(id) == Some("input") { input_type(dom, id) } else { dom.tag(id).unwrap_or("").to_string() };
    let dateish = matches!(t.as_str(), "date" | "month" | "week" | "time" | "datetime-local");
    let len = || forms::value(dom, id).encode_utf16().count();
    if v.value_missing {
        return match t.as_str() {
            "checkbox" => "Please check this box if you want to proceed.",
            "radio" => "Please select one of these options.",
            "select" => "Please select an item in the list.",
            "file" => "Please select a file.",
            _ => "Please fill out this field.",
        }
        .into();
    }
    if v.type_mismatch {
        return if t == "email" { "Please enter an email address." } else { "Please enter a URL." }.into();
    }
    if v.bad_input {
        return if t == "number" { "Please enter a number." } else { "Please enter a valid date or time." }.into();
    }
    if v.pattern_mismatch {
        return match dom.attr(id, "title").filter(|s| !s.trim().is_empty()) {
            Some(title) => format!("Please match the requested format: {}", title.trim()),
            None => "Please match the requested format.".into(),
        };
    }
    if v.too_long {
        return format!("Please shorten this text to {} characters or less (you are currently using {} characters).", dom.attr(id, "maxlength").unwrap_or("").trim(), len());
    }
    if v.too_short {
        return format!("Please use at least {} characters (you are currently using {} characters).", dom.attr(id, "minlength").unwrap_or("").trim(), len());
    }
    if v.range_underflow {
        let min = dom.attr(id, "min").unwrap_or("").trim();
        return if dateish { format!("Please select a value that is no earlier than {min}.") } else { format!("Please select a value that is no less than {min}.") };
    }
    if v.range_overflow {
        let max = dom.attr(id, "max").unwrap_or("").trim();
        return if dateish { format!("Please select a value that is no later than {max}.") } else { format!("Please select a value that is no more than {max}.") };
    }
    if v.step_mismatch {
        return "Please select a valid value.".into();
    }
    String::new()
}

/// Every control of `form` that fails a constraint, in document order.
pub fn invalid_controls(dom: &Dom, state: &FormState, form: NodeId) -> Vec<NodeId> {
    forms::controls(dom, form).into_iter().filter(|&n| will_validate(dom, n) && !validity(dom, state, n).valid()).collect()
}

/// The form is submitted without checking (`novalidate`, or a button with `formnovalidate`).
pub fn no_validate(dom: &Dom, form: NodeId, submitter: Option<NodeId>) -> bool {
    dom.attr(form, "novalidate").is_some() || submitter.is_some_and(|s| dom.attr(s, "formnovalidate").is_some())
}

thread_local! {
    /// A control whose problem should be shown to the user (from reportValidity() or a
    /// blocked submission), picked up by the window loop.
    static REPORT: RefCell<Option<NodeId>> = const { RefCell::new(None) };
}

pub fn request_report(id: NodeId) {
    REPORT.with(|r| *r.borrow_mut() = Some(id));
}

pub fn take_report() -> Option<NodeId> {
    REPORT.with(|r| r.borrow_mut().take())
}

// ---------------------------------------------------------------- script natives

/// Natives behind `forms.js` (validity, setCustomValidity, input.files).
pub mod script {
    use super::*;
    use crate::js::{with_dom, with_dom_mut};
    use boa_engine::object::builtins::{JsArray, JsUint8Array};
    use boa_engine::{Context, JsResult, JsString, JsValue};

    pub type Native = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

    pub const NATIVES: &[(&str, usize, Native)] = &[
        ("__validity", 1, n_validity),
        ("__setCustomValidity", 2, n_set_custom),
        ("__reportValidity", 1, n_report),
        ("__files", 1, n_files),
        ("__fileBytes", 2, n_file_bytes),
    ];

    fn id_arg(a: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<Option<NodeId>> {
        let n = a.get(i).cloned().unwrap_or_default().to_number(ctx)?;
        Ok(with_dom(|d| (n >= 0.0 && (n as usize) < d.nodes.len()).then_some(n as usize)))
    }

    fn n_validity(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = id_arg(a, 0, ctx)? else { return Ok(JsValue::undefined()) };
        let (v, msg, will) = with_dom(|d| {
            let v = validity(d, &d.form_state, id);
            (v, message(d, &d.form_state, id, &v), will_validate(d, id))
        });
        let flags = [v.value_missing, v.type_mismatch, v.pattern_mismatch, v.too_long, v.too_short, v.range_underflow, v.range_overflow, v.step_mismatch, v.bad_input, v.custom_error];
        let mut items: Vec<JsValue> = flags.iter().map(|&f| JsValue::from(f)).collect();
        items.push(JsValue::from(JsString::from(msg)));
        items.push(JsValue::from(will));
        Ok(JsArray::from_iter(items, ctx).into())
    }

    fn n_set_custom(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = id_arg(a, 0, ctx)? else { return Ok(JsValue::undefined()) };
        let msg = a.get(1).cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
        with_dom_mut(|d| {
            if msg.is_empty() {
                d.form_state.custom.remove(&id);
            } else {
                d.form_state.custom.insert(id, msg);
            }
        });
        Ok(JsValue::undefined())
    }

    fn n_report(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        if let Some(id) = id_arg(a, 0, ctx)? {
            request_report(id);
        }
        Ok(JsValue::undefined())
    }

    fn n_files(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = id_arg(a, 0, ctx)? else { return Ok(JsArray::new(ctx)?.into()) };
        let files = with_dom(|d| d.form_state.files.get(&id).cloned().unwrap_or_default());
        let mut out = Vec::new();
        for f in files {
            let item = [JsValue::from(JsString::from(f.name.as_str())), JsValue::from(f.size as f64), JsValue::from(JsString::from(f.mime.as_str())), JsValue::from(f.modified)];
            out.push(JsArray::from_iter(item, ctx).into());
        }
        Ok(JsArray::from_iter(out, ctx).into())
    }

    /// The bytes of a picked file (read when a script asks; files over 256 MB read as empty).
    fn n_file_bytes(_: &JsValue, a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
        let Some(id) = id_arg(a, 0, ctx)? else { return Ok(JsValue::undefined()) };
        let i = a.get(1).cloned().unwrap_or_default().to_number(ctx)? as usize;
        let path = with_dom(|d| d.form_state.files.get(&id).and_then(|f| f.get(i)).filter(|f| f.size <= 256 << 20).map(|f| f.path.clone()));
        let bytes = path.and_then(|p| std::fs::read(p).ok()).unwrap_or_default();
        Ok(JsUint8Array::from_iter(bytes, ctx)?.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(html: &str) -> Vec<Validity> {
        let dom = crate::html::parse(html);
        let mut state = FormState::default();
        let ids: Vec<NodeId> = dom.descendants(Dom::ROOT).into_iter().filter(|&n| matches!(dom.tag(n), Some("input" | "select" | "textarea"))).collect();
        for &i in &ids {
            state.dirty.insert(i);
        }
        ids.into_iter().map(|n| validity(&dom, &state, n)).collect()
    }

    #[test]
    fn required_and_types() {
        let v = check(r#"<input required><input required value=x><input type=email value="a@b.c"><input type=email value="nope">
            <input type=url value="https://x.org"><input type=url value="x.org"><input type=checkbox required><select required><option value="">Pick</option><option>A</option></select>"#);
        assert!(v[0].value_missing && v[1].valid());
        assert!(v[2].valid() && v[3].type_mismatch);
        assert!(v[4].valid() && v[5].type_mismatch);
        assert!(v[6].value_missing && v[7].value_missing);
    }

    #[test]
    fn pattern_length_range_step() {
        let v = check(r#"<input pattern="[0-9]{3}" value="12a"><input pattern="[0-9]{3}" value="123"><input maxlength=3 value="abcd"><input minlength=3 value="ab">
            <input type=number min=5 value=3><input type=number max=5 value=7><input type=number step=2 value=3><input type=number value="1e"><input type=number step=0.1 value=0.3>
            <input type=date min="2024-01-10" value="2024-01-09"><input type=time step=900 value="10:15"><input type=time step=900 value="10:20">"#);
        assert!(v[0].pattern_mismatch && v[1].valid());
        assert!(v[2].too_long && v[3].too_short);
        assert!(v[4].range_underflow && v[5].range_overflow && v[6].step_mismatch && v[7].bad_input && v[8].valid());
        assert!(v[9].range_underflow && v[10].valid() && v[11].step_mismatch);
    }

    #[test]
    fn radio_groups_and_disabled() {
        let v = check(r#"<form><input type=radio name=a required><input type=radio name=a><input type=radio name=b required><input type=radio name=b checked><input disabled required></form>"#);
        assert!(v[0].value_missing && v[1].value_missing && v[2].valid() && v[3].valid() && v[4].valid());
    }

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert!(parse_date("2023-02-29").is_none());
        assert_eq!(Numeric::Week.parse("1970-W01"), Some(-259_200_000.0));
        assert!(parse_week("2020-W53").is_some() && parse_week("2021-W53").is_none());
    }
}
