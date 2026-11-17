//! CSS transitions and animations (`@keyframes`).
//!
//! Animated values are worked out during the cascade: `style::compute` calls `begin_pass`,
//! then `animate` for each element with an `animation` or `transition` (its result is applied
//! like ordinary declarations), then `end_pass`. Values are interpolated as text: numbers with
//! their units (and colors, turned into `rgba()`) are blended when both ends have the same
//! shape, otherwise the value flips halfway, as CSS does for discrete properties.
//!
//! While anything is moving, `active()` is true and the window restyles every frame.
//! Each page keeps its own timeline (`set_current`). For screenshots,
//! `IRONSWIFT_ANIM_TIME=<ms>` draws every animation as if it had been running that long.

use crate::dom::NodeId;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

// ------------------------------------------------------------------------------- timing

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    Linear,
    Bezier(f32, f32, f32, f32),
    /// steps(n, jump-start?)
    Steps(u32, bool),
}

impl Easing {
    pub fn parse(s: &str) -> Option<Easing> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "linear" => Easing::Linear,
            "ease" => Easing::Bezier(0.25, 0.1, 0.25, 1.0),
            "ease-in" => Easing::Bezier(0.42, 0.0, 1.0, 1.0),
            "ease-out" => Easing::Bezier(0.0, 0.0, 0.58, 1.0),
            "ease-in-out" => Easing::Bezier(0.42, 0.0, 0.58, 1.0),
            "step-start" => Easing::Steps(1, true),
            "step-end" => Easing::Steps(1, false),
            _ => {
                let (name, rest) = s.split_once('(')?;
                let args: Vec<&str> = rest.trim_end_matches(')').split(',').map(str::trim).collect();
                match name.trim() {
                    "cubic-bezier" if args.len() == 4 => {
                        let n: Vec<f32> = args.iter().filter_map(|a| a.parse().ok()).collect();
                        if n.len() != 4 {
                            return None;
                        }
                        Easing::Bezier(n[0].clamp(0.0, 1.0), n[1], n[2].clamp(0.0, 1.0), n[3])
                    }
                    "steps" => {
                        let n: u32 = args.first()?.parse().ok().filter(|&n| n > 0)?;
                        let start = matches!(args.get(1).copied(), Some("start" | "jump-start"));
                        Easing::Steps(n, start)
                    }
                    "linear" => Easing::Linear,
                    _ => return None,
                }
            }
        })
    }

    /// Eased progress for input progress `t` (0..1).
    pub fn at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match *self {
            Easing::Linear => t,
            Easing::Steps(n, start) => {
                let n = n as f32;
                let s = if start { (t * n).floor() + 1.0 } else { (t * n).floor() };
                (s / n).clamp(0.0, 1.0)
            }
            Easing::Bezier(x1, y1, x2, y2) => {
                let bez = |a: f32, b: f32, u: f32| 3.0 * a * u * (1.0 - u) * (1.0 - u) + 3.0 * b * u * u * (1.0 - u) + u * u * u;
                // Solve x(u) = t by bisection (x is monotonic for x1, x2 in 0..1).
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..30 {
                    let mid = (lo + hi) / 2.0;
                    if bez(x1, x2, mid) < t {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                bez(y1, y2, (lo + hi) / 2.0)
            }
        }
    }
}

fn time_secs(s: &str) -> Option<f32> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(v) = s.strip_suffix("ms") {
        return v.parse::<f32>().ok().map(|v| v / 1000.0);
    }
    s.strip_suffix('s')?.parse().ok()
}

// ------------------------------------------------------------------------------- properties

/// An element's `animation-*` and `transition-*` lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimSpec {
    pub names: Vec<String>,
    pub durations: Vec<f32>,
    pub delays: Vec<f32>,
    pub timings: Vec<Easing>,
    /// Iteration counts (infinity for `infinite`).
    pub iterations: Vec<f32>,
    /// 0 normal, 1 reverse, 2 alternate, 3 alternate-reverse
    pub directions: Vec<u8>,
    /// bit 1 forwards, bit 2 backwards
    pub fills: Vec<u8>,
    pub paused: Vec<bool>,
    pub t_props: Vec<String>,
    pub t_durations: Vec<f32>,
    pub t_delays: Vec<f32>,
    pub t_timings: Vec<Easing>,
}

fn nth<T: Copy>(v: &[T], i: usize, default: T) -> T {
    if v.is_empty() { default } else { v[i % v.len()] }
}

pub fn is_anim_prop(name: &str) -> bool {
    name.starts_with("animation") || name.starts_with("transition") || name.starts_with("-webkit-animation") || name.starts_with("-webkit-transition")
}

fn list(v: &str) -> Vec<String> {
    crate::css::split_top(v, ',').into_iter().map(|s| s.trim().to_string()).collect()
}

/// Apply one `animation…` / `transition…` declaration.
pub fn apply_prop(spec: &mut Option<Rc<AnimSpec>>, name: &str, value: &str) {
    let name = name.strip_prefix("-webkit-").unwrap_or(name);
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let s = Rc::make_mut(spec.get_or_insert_with(Default::default));
    let items = list(&lower);
    match name {
        "animation-name" => s.names = list(value),
        "animation-duration" => s.durations = items.iter().map(|t| time_secs(t).unwrap_or(0.0)).collect(),
        "animation-delay" => s.delays = items.iter().map(|t| time_secs(t).unwrap_or(0.0)).collect(),
        "animation-timing-function" => s.timings = items.iter().map(|t| Easing::parse(t).unwrap_or(Easing::Linear)).collect(),
        "animation-iteration-count" => s.iterations = items.iter().map(|t| if t == "infinite" { f32::INFINITY } else { t.parse().unwrap_or(1.0) }).collect(),
        "animation-direction" => s.directions = items.iter().map(|t| direction(t).unwrap_or(0)).collect(),
        "animation-fill-mode" => s.fills = items.iter().map(|t| fill(t).unwrap_or(0)).collect(),
        "animation-play-state" => s.paused = items.iter().map(|t| t == "paused").collect(),
        "animation" => {
            *s = AnimSpec { t_props: s.t_props.clone(), t_durations: s.t_durations.clone(), t_delays: s.t_delays.clone(), t_timings: s.t_timings.clone(), ..Default::default() };
            for (i, part) in list(value).iter().enumerate() {
                let (mut times, mut name) = (Vec::new(), "none".to_string());
                let (mut timing, mut iters, mut dir, mut fil, mut paused) = (Easing::Bezier(0.25, 0.1, 0.25, 1.0), 1.0, 0u8, 0u8, false);
                for tok in crate::css::split_top(part, ' ') {
                    let tok = tok.trim();
                    let tl = tok.to_ascii_lowercase();
                    if tok.is_empty() {
                        continue;
                    }
                    if let Some(t) = time_secs(&tl) {
                        times.push(t);
                    } else if let Some(e) = Easing::parse(&tl) {
                        timing = e;
                    } else if tl == "infinite" {
                        iters = f32::INFINITY;
                    } else if let Ok(n) = tl.parse::<f32>() {
                        iters = n;
                    } else if let Some(d) = direction(&tl) {
                        dir = d;
                    } else if let Some(f) = fill(&tl) {
                        fil = f;
                    } else if tl == "paused" || tl == "running" {
                        paused = tl == "paused";
                    } else {
                        name = tok.to_string();
                    }
                }
                let _ = i;
                s.names.push(name);
                s.durations.push(times.first().copied().unwrap_or(0.0));
                s.delays.push(times.get(1).copied().unwrap_or(0.0));
                s.timings.push(timing);
                s.iterations.push(iters);
                s.directions.push(dir);
                s.fills.push(fil);
                s.paused.push(paused);
            }
        }
        "transition-property" => s.t_props = items,
        "transition-duration" => s.t_durations = items.iter().map(|t| time_secs(t).unwrap_or(0.0)).collect(),
        "transition-delay" => s.t_delays = items.iter().map(|t| time_secs(t).unwrap_or(0.0)).collect(),
        "transition-timing-function" => s.t_timings = items.iter().map(|t| Easing::parse(t).unwrap_or(Easing::Linear)).collect(),
        "transition" => {
            s.t_props.clear();
            s.t_durations.clear();
            s.t_delays.clear();
            s.t_timings.clear();
            for part in &items {
                let (mut times, mut prop, mut timing) = (Vec::new(), "all".to_string(), Easing::Bezier(0.25, 0.1, 0.25, 1.0));
                for tok in crate::css::split_top(part, ' ') {
                    let tok = tok.trim();
                    if tok.is_empty() {
                        continue;
                    }
                    if let Some(t) = time_secs(tok) {
                        times.push(t);
                    } else if let Some(e) = Easing::parse(tok) {
                        timing = e;
                    } else {
                        prop = tok.to_string();
                    }
                }
                s.t_props.push(prop);
                s.t_durations.push(times.first().copied().unwrap_or(0.0));
                s.t_delays.push(times.get(1).copied().unwrap_or(0.0));
                s.t_timings.push(timing);
            }
        }
        _ => {}
    }
}

fn direction(t: &str) -> Option<u8> {
    Some(match t {
        "normal" => 0,
        "reverse" => 1,
        "alternate" => 2,
        "alternate-reverse" => 3,
        _ => return None,
    })
}

fn fill(t: &str) -> Option<u8> {
    Some(match t {
        "none" => 0,
        "forwards" => 1,
        "backwards" => 2,
        "both" => 3,
        _ => return None,
    })
}

// ------------------------------------------------------------------------------- @keyframes

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub offset: f32,
    pub decls: Vec<(String, String)>,
    pub easing: Option<Easing>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyframes {
    pub name: String,
    pub frames: Vec<Frame>,
}

fn matching_brace(b: &[u8], open: usize) -> usize {
    let mut depth = 0;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    b.len()
}

/// Every `@keyframes` rule in a style sheet's text (comments already removed).
pub fn find_keyframes(src: &str) -> Vec<Keyframes> {
    let mut out = Vec::new();
    let lower = src.to_ascii_lowercase();
    let b = src.as_bytes();
    let mut from = 0;
    while let Some(p) = lower[from..].find("keyframes") {
        let at = from + p;
        from = at + 9;
        let prefix_ok = lower[..at].ends_with('@') || lower[..at].ends_with("@-webkit-") || lower[..at].ends_with("@-moz-");
        if !prefix_ok {
            continue;
        }
        let Some(open) = src[from..].find('{').map(|o| from + o) else { break };
        let name = src[from..open].trim().trim_matches(|c| c == '"' || c == '\'').to_string();
        let close = matching_brace(b, open);
        let body = &src[open + 1..close.min(src.len())];
        from = close.min(src.len());
        if name.is_empty() {
            continue;
        }
        let mut frames: Vec<Frame> = Vec::new();
        let bb = body.as_bytes();
        let mut i = 0;
        while let Some(o) = body[i..].find('{').map(|o| i + o) {
            let sel = body[i..o].trim().to_ascii_lowercase();
            let c = matching_brace(bb, o);
            let decl_src = &body[o + 1..c.min(body.len())];
            i = (c + 1).min(body.len());
            let decls = crate::css::parse_declarations(decl_src);
            let easing = decls.iter().rev().find(|d| d.name == "animation-timing-function").and_then(|d| Easing::parse(&d.value));
            let decls: Vec<(String, String)> = decls.into_iter().filter(|d| d.name != "animation-timing-function").map(|d| (d.name, d.value)).collect();
            for s in sel.split(',') {
                let off = match s.trim() {
                    "from" => Some(0.0),
                    "to" => Some(1.0),
                    p => p.strip_suffix('%').and_then(|v| v.trim().parse::<f32>().ok()).map(|v| v / 100.0),
                };
                if let Some(off) = off.filter(|o| (0.0..=1.0).contains(o)) {
                    // Several rules for the same offset merge.
                    match frames.iter_mut().find(|f| (f.offset - off).abs() < 1e-6) {
                        Some(f) => {
                            f.decls.extend(decls.iter().cloned());
                            if easing.is_some() {
                                f.easing = easing;
                            }
                        }
                        None => frames.push(Frame { offset: off, decls: decls.clone(), easing }),
                    }
                }
            }
        }
        frames.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap_or(std::cmp::Ordering::Equal));
        out.push(Keyframes { name, frames });
    }
    out
}

// ------------------------------------------------------------------------------- interpolation

/// A value with colors written as `rgba(r, g, b, a)` so they blend channel by channel.
fn normalize(v: &str) -> String {
    let v = v.trim();
    let words = crate::css::split_top(v, ' ');
    let mut out = Vec::new();
    for w in words {
        let w = w.trim();
        if w.is_empty() {
            continue;
        }
        let is_colorish = w.starts_with('#') || w.starts_with("rgb") || w.starts_with("hsl") || w.starts_with("hwb") || w.starts_with("lab") || w.starts_with("lch") || w.starts_with("okl") || w.chars().all(|c| c.is_ascii_alphabetic());
        match is_colorish.then(|| crate::style::parse_color(w)).flatten() {
            Some(c) => out.push(format!("rgba({}, {}, {}, {})", c.r, c.g, c.b, c.a as f32 / 255.0)),
            None => out.push(w.to_string()),
        }
    }
    out.join(" ")
}

/// The no-op version of a list of functions, e.g. `translateX(10px) scale(2)` →
/// `translateX(0px) scale(1)`, for animating to or from `none`.
fn identity_of(v: &str) -> Option<String> {
    let mut out = Vec::new();
    for f in crate::css::split_top(v, ' ') {
        let f = f.trim();
        if f.is_empty() {
            continue;
        }
        let (name, args) = f.split_once('(')?;
        let args = args.trim_end_matches(')');
        let one = matches!(name, "scale" | "scalex" | "scaley" | "scalez" | "scale3d" | "brightness" | "contrast" | "saturate" | "opacity");
        let zeroed: Vec<String> = crate::css::split_top(args, ',')
            .iter()
            .map(|a| {
                let a = a.trim();
                let unit: String = a.chars().skip_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
                if one { "1".to_string() } else { format!("0{unit}") }
            })
            .collect();
        if name == "matrix" {
            out.push("matrix(1, 0, 0, 1, 0, 0)".to_string());
        } else {
            out.push(format!("{name}({})", zeroed.join(", ")));
        }
    }
    Some(out.join(" "))
}

/// Split into text pieces and numbers-with-units: "translate(10px, 5%)" →
/// (["translate(", ", ", ")"], [(10, "px"), (5, "%")]).
fn split_numbers(v: &str) -> (Vec<String>, Vec<(f32, String)>) {
    let c: Vec<char> = v.chars().collect();
    let mut skel = vec![String::new()];
    let mut nums = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let prev_ok = i == 0 || !(c[i - 1].is_ascii_alphanumeric() || c[i - 1] == '_' || c[i - 1] == '-' && i >= 2 && c[i - 2].is_ascii_alphanumeric());
        let starts = c[i].is_ascii_digit() || c[i] == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()) || (c[i] == '-' || c[i] == '+') && c.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '.');
        if prev_ok && starts {
            let mut j = i + 1;
            while j < c.len() && (c[j].is_ascii_digit() || c[j] == '.') {
                j += 1;
            }
            let num: f32 = c[i..j].iter().collect::<String>().parse().unwrap_or(0.0);
            let mut k = j;
            while k < c.len() && (c[k].is_ascii_alphabetic() || c[k] == '%') {
                k += 1;
            }
            nums.push((num, c[j..k].iter().collect::<String>().to_ascii_lowercase()));
            skel.push(String::new());
            i = k;
        } else {
            skel.last_mut().unwrap().push(c[i]);
            i += 1;
        }
    }
    (skel, nums)
}

fn fmt_num(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The value `t` of the way from `a` to `b`.
pub fn interpolate(a: &str, b: &str, t: f32) -> String {
    if a == b {
        return a.to_string();
    }
    let (mut na, mut nb) = (normalize(a).to_ascii_lowercase(), normalize(b).to_ascii_lowercase());
    if na == "none" && nb.contains('(') {
        if let Some(id) = identity_of(&nb) {
            na = id;
        }
    } else if nb == "none" && na.contains('(') {
        if let Some(id) = identity_of(&na) {
            nb = id;
        }
    }
    let (sa, xa) = split_numbers(&na);
    let (sb, xb) = split_numbers(&nb);
    let discrete = || if t < 0.5 { a.to_string() } else { b.to_string() };
    if sa != sb || xa.len() != xb.len() {
        return discrete();
    }
    let mut out = String::new();
    // Inside rgba(): which channel comes next (color channels are whole numbers).
    let mut chan: Option<usize> = None;
    for (i, s) in sa.iter().enumerate() {
        out.push_str(s);
        if s.ends_with("rgba(") {
            chan = Some(0);
        } else if s.contains(')') {
            chan = None;
        }
        if let (Some((va, ua)), Some((vb, ub))) = (xa.get(i), xb.get(i)) {
            let unit = if ua == ub || *vb == 0.0 && ub.is_empty() {
                ua
            } else if *va == 0.0 && ua.is_empty() {
                ub
            } else {
                return discrete();
            };
            let v = va + (vb - va) * t;
            out.push_str(&fmt_num(if chan.is_some_and(|c| c < 3) { v.round() } else { v }));
            out.push_str(unit);
            chan = chan.map(|c| c + 1);
        }
    }
    out
}

/// Values used when a keyframe or transition has nothing declared for a property.
fn initial(prop: &str) -> Option<&'static str> {
    Some(match prop {
        "opacity" | "fill-opacity" | "stroke-opacity" | "stop-opacity" | "stroke-width" => "1",
        "fill" | "stop-color" => "black",
        "stroke-dashoffset" => "0",
        "transform" | "filter" | "backdrop-filter" | "box-shadow" | "text-shadow" => "none",
        "background-color" | "border-color" | "outline-color" => "transparent",
        "letter-spacing" | "word-spacing" | "margin-top" | "margin-bottom" | "margin-left" | "margin-right" | "padding-top" | "padding-bottom" | "padding-left" | "padding-right" | "border-radius" => "0px",
        "left" | "top" | "right" | "bottom" | "width" | "height" => "auto",
        _ => return None,
    })
}

// ------------------------------------------------------------------------------- timelines

struct Running {
    from: String,
    to: String,
    start: Instant,
    delay: f32,
    dur: f32,
    easing: Easing,
    /// Has transitionstart fired (the delay is over)?
    begun: bool,
}

impl Running {
    fn value(&self, now: Instant) -> (String, bool) {
        let el = now.saturating_duration_since(self.start).as_secs_f32() - self.delay;
        if el < 0.0 {
            return (self.from.clone(), false);
        }
        let p = if self.dur > 0.0 { el / self.dur } else { 1.0 };
        if p >= 1.0 {
            return (self.to.clone(), true);
        }
        (interpolate(&self.from, &self.to, self.easing.at(p)), false)
    }
}

#[derive(Default)]
struct Timeline {
    /// When each (element, animation name) started, and its elapsed time while paused.
    started: HashMap<(NodeId, String), (Instant, Option<f32>)>,
    /// The last value each transitioned property was given.
    last: HashMap<(NodeId, String), String>,
    running: HashMap<(NodeId, String), Running>,
    /// Each animation's phase (0 before, 1 active, 2 after) and iteration at the last pass,
    /// for animationstart/iteration/end.
    phases: HashMap<(NodeId, String), (u8, f32)>,
    /// Elements that had any state, for the next pass.
    nodes: HashSet<NodeId>,
}

thread_local! {
    static TIMELINES: RefCell<Vec<(u64, Timeline)>> = const { RefCell::new(Vec::new()) };
    static CURRENT: Cell<Option<u64>> = const { Cell::new(None) };
    static PASS: RefCell<Option<Pass>> = const { RefCell::new(None) };
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static EVENTS: RefCell<Vec<AnimEvent>> = const { RefCell::new(Vec::new()) };
}

/// An animation or transition event for the page's scripts (animationstart, transitionend…).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimEvent {
    /// The page whose timeline it came from (see `set_current`).
    pub key: u64,
    pub node: NodeId,
    pub kind: &'static str,
    /// The animation name, or the transitioned property.
    pub name: String,
    /// Seconds into the animation or transition (`elapsedTime`).
    pub elapsed: f32,
}

impl AnimEvent {
    /// The event's fields as a JavaScript object literal.
    pub fn init(&self) -> String {
        let field = if self.kind.starts_with("transition") { "propertyName" } else { "animationName" };
        let name: String = self.name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
        format!("{{{field}:\"{name}\",elapsedTime:{},pseudoElement:\"\"}}", self.elapsed)
    }
}

/// The events style passes produced since the last call, oldest first.
pub fn take_events() -> Vec<AnimEvent> {
    EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()))
}

struct Pass {
    key: Option<u64>,
    now: Instant,
    fixed: Option<f32>,
    keyframes: HashMap<String, Rc<Keyframes>>,
    seen_anims: HashSet<(NodeId, String)>,
    seen_nodes: HashSet<NodeId>,
    active: bool,
    timeline: Timeline,
    /// (node, kind, name, elapsed) for events this pass.
    events: Vec<(NodeId, &'static str, String, f32)>,
}

/// Use page `key`'s timeline for the next style pass (pages keep separate timelines).
/// Passes without this (first styling, frames, printing) start from a blank timeline and
/// keep nothing.
pub fn set_current(key: u64) {
    CURRENT.with(|c| c.set(Some(key)));
}

/// Did the last style pass have anything moving (or waiting to start)?
pub fn active() -> bool {
    ACTIVE.with(Cell::get)
}

fn fixed_time() -> Option<f32> {
    std::env::var("IRONSWIFT_ANIM_TIME").ok().and_then(|v| v.parse::<f32>().ok()).map(|ms| ms / 1000.0)
}

pub fn begin_pass(sheets: &[crate::css::Stylesheet]) {
    let key = CURRENT.with(Cell::take);
    let timeline = key
        .and_then(|key| {
            TIMELINES.with(|t| {
                let mut t = t.borrow_mut();
                t.iter().position(|(k, _)| *k == key).map(|i| t.remove(i).1)
            })
        })
        .unwrap_or_default();
    let mut keyframes = HashMap::new();
    for s in sheets {
        for k in &s.keyframes {
            keyframes.insert(k.name.clone(), Rc::new(k.clone()));
        }
    }
    PASS.with(|p| {
        *p.borrow_mut() = Some(Pass { key, now: Instant::now(), fixed: fixed_time(), keyframes, seen_anims: HashSet::new(), seen_nodes: HashSet::new(), active: false, timeline, events: Vec::new() })
    });
}

pub fn end_pass() {
    let Some(mut pass) = PASS.with(|p| p.borrow_mut().take()) else { return };
    let tl = &mut pass.timeline;
    tl.started.retain(|k, _| pass.seen_anims.contains(k));
    tl.phases.retain(|k, _| pass.seen_anims.contains(k));
    tl.last.retain(|(n, _), _| pass.seen_nodes.contains(n));
    tl.running.retain(|(n, _), _| pass.seen_nodes.contains(n));
    tl.nodes = pass.seen_nodes;
    if let Some(key) = pass.key {
        let evs = std::mem::take(&mut pass.events);
        EVENTS.with(|e| e.borrow_mut().extend(evs.into_iter().map(|(node, kind, name, elapsed)| AnimEvent { key, node, kind, name, elapsed })));
    }
    let Some(key) = pass.key else {
        // A one-off pass: it can start the clock (active) but never stops it.
        if pass.active {
            ACTIVE.with(|a| a.set(true));
        }
        return;
    };
    ACTIVE.with(|a| a.set(pass.active));
    TIMELINES.with(|t| {
        let mut t = t.borrow_mut();
        t.push((key, pass.timeline));
        // Keep the timelines of the last few pages only.
        while t.len() > 16 {
            t.remove(0);
        }
    });
}

/// Does this element need `animate` even without animation properties (a transition that
/// was running when its `transition` declaration went away)?
pub fn has_state(id: NodeId) -> bool {
    PASS.with(|p| p.borrow().as_ref().is_some_and(|p| p.timeline.nodes.contains(&id)))
}

/// The animated property values for element `id` right now, to apply over its cascaded
/// values. `decls` are the cascaded declarations that won (name, value).
pub fn animate(id: NodeId, spec: Option<&AnimSpec>, decls: &[(String, String)]) -> Vec<(String, String)> {
    PASS.with(|p| {
        let mut p = p.borrow_mut();
        let Some(pass) = p.as_mut() else { return Vec::new() };
        pass.seen_nodes.insert(id);
        let mut out: Vec<(String, String)> = Vec::new();
        let underlying = |prop: &str| decls.iter().rev().find(|(n, _)| n == prop).map(|(_, v)| v.clone()).or_else(|| initial(prop).map(str::to_string));
        // Transitions.
        if let Some(s) = spec.filter(|s| !s.t_props.is_empty()) {
            let mut props: Vec<String> = decls.iter().map(|(n, _)| n.clone()).collect();
            props.extend(pass.timeline.last.keys().filter(|(n, _)| *n == id).map(|(_, p)| p.clone()));
            props.sort();
            props.dedup();
            for prop in props {
                let Some(i) = s.t_props.iter().rposition(|t| t == "all" || *t == prop || prop.starts_with(&format!("{t}-"))) else { continue };
                let (dur, delay) = (nth(&s.t_durations, i, 0.0), nth(&s.t_delays, i, 0.0));
                let key = (id, prop.clone());
                let Some(target) = underlying(&prop) else { continue };
                if dur <= 0.0 && delay <= 0.0 {
                    if pass.timeline.running.remove(&key).is_some() {
                        pass.events.push((id, "transitioncancel", prop.clone(), 0.0));
                    }
                    pass.timeline.last.insert(key, target);
                    continue;
                }
                if let Some(prev) = pass.timeline.last.get(&key).filter(|v| **v != target).cloned() {
                    let from = match pass.timeline.running.get(&key) {
                        Some(r) => {
                            let el = pass.now.saturating_duration_since(r.start).as_secs_f32() - r.delay;
                            pass.events.push((id, "transitioncancel", prop.clone(), el.clamp(0.0, r.dur)));
                            r.value(pass.now).0
                        }
                        None => prev,
                    };
                    pass.timeline.running.insert(key.clone(), Running { from, to: target.clone(), start: pass.now, delay, dur, easing: nth(&s.t_timings, i, Easing::Linear), begun: false });
                    pass.events.push((id, "transitionrun", prop.clone(), 0.0));
                }
                pass.timeline.last.insert(key.clone(), target);
                if let Some(r) = pass.timeline.running.get_mut(&key) {
                    let (v, done) = r.value(pass.now);
                    let el = pass.now.saturating_duration_since(r.start).as_secs_f32() - r.delay;
                    if !r.begun && el >= 0.0 {
                        r.begun = true;
                        pass.events.push((id, "transitionstart", prop.clone(), 0.0));
                    }
                    if done {
                        let d = r.dur;
                        pass.timeline.running.remove(&key);
                        pass.events.push((id, "transitionend", prop.clone(), d));
                    } else {
                        pass.active = true;
                        out.push((prop, v));
                    }
                }
            }
        } else {
            for ((n, prop), _) in pass.timeline.running.iter() {
                if *n == id {
                    pass.events.push((id, "transitioncancel", prop.clone(), 0.0));
                }
            }
            pass.timeline.running.retain(|(n, _), _| *n != id);
            pass.timeline.last.retain(|(n, _), _| *n != id);
        }
        // Animations (later ones win, and they beat transitions).
        let Some(s) = spec else { return out };
        for (i, name) in s.names.iter().enumerate() {
            let Some(kf) = pass.keyframes.get(name).cloned() else { continue };
            let key = (id, name.clone());
            pass.seen_anims.insert(key.clone());
            let now = pass.now;
            let entry = pass.timeline.started.entry(key).or_insert((now, None));
            let paused = nth(&s.paused, i, false);
            let delay = nth(&s.delays, i, 0.0);
            let mut el = match (pass.fixed, paused, entry.1) {
                (Some(t), _, _) => t,
                (None, true, Some(e)) => e,
                (None, true, None) => {
                    let e = now.saturating_duration_since(entry.0).as_secs_f32();
                    entry.1 = Some(e);
                    e
                }
                (None, false, Some(e)) => {
                    *entry = (now - std::time::Duration::from_secs_f32(e.max(0.0)), None);
                    e
                }
                (None, false, None) => now.saturating_duration_since(entry.0).as_secs_f32(),
            };
            el -= delay;
            let dur = nth(&s.durations, i, 0.0);
            let iters = nth(&s.iterations, i, 1.0).max(0.0);
            let fillm = nth(&s.fills, i, 0);
            let dir = nth(&s.directions, i, 0);
            let total = dur * iters;
            // animationstart, animationiteration and animationend as the phase changes.
            let phase: u8 = if el < 0.0 { 0 } else if el >= total || dur <= 0.0 { 2 } else { 1 };
            let it_now = if phase == 1 { (el / dur).floor() } else { 0.0 };
            let pkey = (id, name.clone());
            let prev = pass.timeline.phases.get(&pkey).copied();
            if phase >= 1 && prev.is_none_or(|p| p.0 == 0) {
                pass.events.push((id, "animationstart", name.clone(), 0.0));
            }
            if let (1, Some((1, pi))) = (phase, prev) {
                if it_now > pi {
                    pass.events.push((id, "animationiteration", name.clone(), it_now * dur));
                }
            }
            if phase == 2 && prev.is_none_or(|p| p.0 < 2) {
                pass.events.push((id, "animationend", name.clone(), if total.is_finite() { total.max(0.0) } else { 0.0 }));
            }
            pass.timeline.phases.insert(pkey, (phase, it_now));
            let (iter, p) = if el < 0.0 {
                if !paused && pass.fixed.is_none() {
                    pass.active = true;
                }
                if fillm & 2 == 0 {
                    continue;
                }
                (0.0, 0.0)
            } else if el >= total || dur <= 0.0 {
                if fillm & 1 == 0 {
                    continue;
                }
                if iters == 0.0 {
                    (0.0, 0.0)
                } else if iters.fract() == 0.0 {
                    (iters - 1.0, 1.0)
                } else {
                    (iters.floor(), iters.fract())
                }
            } else {
                if !paused && pass.fixed.is_none() {
                    pass.active = true;
                }
                ((el / dur).floor(), (el / dur).fract())
            };
            let reversed = match dir {
                1 => true,
                2 => iter as u64 % 2 == 1,
                3 => iter as u64 % 2 == 0,
                _ => false,
            };
            let p = if reversed { 1.0 - p } else { p };
            let timing = nth(&s.timings, i, Easing::Bezier(0.25, 0.1, 0.25, 1.0));
            // Every property any keyframe mentions.
            let mut props: Vec<&String> = kf.frames.iter().flat_map(|f| f.decls.iter().map(|(n, _)| n)).collect();
            props.sort();
            props.dedup();
            for prop in props {
                let mut stops: Vec<(f32, String, Easing)> = kf
                    .frames
                    .iter()
                    .filter_map(|f| f.decls.iter().rev().find(|(n, _)| n == prop).map(|(_, v)| (f.offset, v.clone(), f.easing.unwrap_or(timing))))
                    .collect();
                let base = underlying(prop);
                if stops.first().is_some_and(|s| s.0 > 0.0) {
                    let v = base.clone().unwrap_or_else(|| stops[0].1.clone());
                    stops.insert(0, (0.0, v, timing));
                }
                if stops.last().is_some_and(|s| s.0 < 1.0) {
                    let v = base.clone().unwrap_or_else(|| stops[stops.len() - 1].1.clone());
                    stops.push((1.0, v, timing));
                }
                let k = stops.iter().rposition(|s| s.0 <= p).unwrap_or(0).min(stops.len().saturating_sub(2));
                let v = match (stops.get(k), stops.get(k + 1)) {
                    (Some(a), Some(b)) if b.0 > a.0 => interpolate(&a.1, &b.1, a.2.at((p - a.0) / (b.0 - a.0))),
                    (Some(a), _) => a.1.clone(),
                    _ => continue,
                };
                out.retain(|(n, _)| n != prop);
                out.push((prop.clone(), v));
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_numbers_colors_and_transforms() {
        assert_eq!(interpolate("0px", "100px", 0.25), "25px");
        assert_eq!(interpolate("0", "1", 0.5), "0.5");
        assert_eq!(interpolate("red", "blue", 0.5), "rgba(128, 0, 128, 1)");
        assert_eq!(interpolate("none", "translateX(100px) scale(3)", 0.5), "translatex(50px) scale(2)");
        assert_eq!(interpolate("rotate(0deg)", "rotate(1turn)", 0.5), "rotate(1turn)");
        assert_eq!(interpolate("block", "none", 0.4), "block");
        assert_eq!(interpolate("block", "none", 0.6), "none");
        assert_eq!(interpolate("translate3d(0px, 0px, 0px)", "translate3d(10px, 20px, 0px)", 0.5), "translate3d(5px, 10px, 0px)");
    }

    #[test]
    fn easing_curves() {
        let e = Easing::parse("ease-in-out").unwrap();
        assert!((e.at(0.5) - 0.5).abs() < 0.01);
        assert!(e.at(0.1) < 0.1);
        assert_eq!(Easing::parse("steps(4)").unwrap().at(0.3), 0.25);
        assert_eq!(Easing::parse("steps(4, start)").unwrap().at(0.3), 0.5);
        assert_eq!(Easing::parse("linear").unwrap().at(0.3), 0.3);
    }

    #[test]
    fn parses_keyframes_and_shorthands() {
        let k = find_keyframes("@keyframes fade { from { opacity: 0 } 50%, 75% { opacity: .8; animation-timing-function: linear } to { opacity: 1 } } p{}");
        assert_eq!(k.len(), 1);
        assert_eq!(k[0].name, "fade");
        assert_eq!(k[0].frames.iter().map(|f| f.offset).collect::<Vec<_>>(), vec![0.0, 0.5, 0.75, 1.0]);
        assert_eq!(k[0].frames[1].easing, Some(Easing::Linear));
        let mut s = None;
        apply_prop(&mut s, "animation", "fade 2s ease-in 500ms infinite alternate both");
        let s2 = s.clone().unwrap();
        assert_eq!(s2.names, vec!["fade"]);
        assert_eq!((s2.durations[0], s2.delays[0], s2.iterations[0], s2.directions[0], s2.fills[0]), (2.0, 0.5, f32::INFINITY, 2, 3));
        apply_prop(&mut s, "transition", "opacity .3s, transform 1s ease-out 100ms");
        let s3 = s.unwrap();
        assert_eq!(s3.t_props, vec!["opacity", "transform"]);
        assert_eq!(s3.t_durations, vec![0.3, 1.0]);
        assert_eq!(s3.t_delays, vec![0.0, 0.1]);
    }

    #[test]
    fn animation_runs_on_the_timeline() {
        set_current(9_999);
        let mut sheet = crate::css::parse("@keyframes slide { from { margin-left: 0px } to { margin-left: 100px } }");
        sheet.keyframes = find_keyframes("@keyframes slide { from { margin-left: 0px } to { margin-left: 100px } }");
        let mut spec = None;
        apply_prop(&mut spec, "animation", "slide 10s linear");
        begin_pass(std::slice::from_ref(&sheet));
        let v = animate(5, spec.as_deref(), &[]);
        end_pass();
        assert_eq!(v, vec![("margin-left".to_string(), "0px".to_string())]);
        assert!(active());
    }

    #[test]
    fn transition_starts_when_value_changes() {
        set_current(9_998);
        let mut spec = None;
        apply_prop(&mut spec, "transition", "opacity 10s linear");
        let d = |v: &str| vec![("opacity".to_string(), v.to_string())];
        begin_pass(&[]);
        assert!(animate(3, spec.as_deref(), &d("1")).is_empty());
        end_pass();
        set_current(9_998);
        begin_pass(&[]);
        let v = animate(3, spec.as_deref(), &d("0"));
        end_pass();
        assert_eq!(v.len(), 1);
        assert!(v[0].1.parse::<f32>().unwrap() > 0.99);
        assert!(active());
    }

    #[test]
    fn animation_and_transition_events() {
        let _ = take_events();
        let src = "@keyframes pop { from { opacity: 0 } to { opacity: 1 } }";
        let mut sheet = crate::css::parse(src);
        sheet.keyframes = find_keyframes(src);
        let mut spec = None;
        // Iterations long enough that a slow test machine oversleeping doesn't skip one.
        apply_prop(&mut spec, "animation", "pop 0.4s linear 2");
        let pass = |spec: &Option<Rc<AnimSpec>>| {
            set_current(9_997);
            begin_pass(std::slice::from_ref(&sheet));
            animate(7, spec.as_deref(), &[]);
            end_pass();
            take_events().into_iter().map(|e| e.kind).collect::<Vec<_>>()
        };
        assert_eq!(pass(&spec), vec!["animationstart"]);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(pass(&spec), vec!["animationiteration"]);
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert_eq!(pass(&spec), vec!["animationend"]);
        assert!(pass(&spec).is_empty());

        let mut t = None;
        apply_prop(&mut t, "transition", "opacity 0.01s linear");
        let d = |v: &str| vec![("opacity".to_string(), v.to_string())];
        let tpass = |v: &str| {
            set_current(9_996);
            begin_pass(&[]);
            animate(4, t.as_deref(), &d(v));
            end_pass();
            take_events()
        };
        assert!(tpass("1").is_empty());
        let started: Vec<_> = tpass("0").into_iter().map(|e| e.kind).collect();
        assert_eq!(started, vec!["transitionrun", "transitionstart"]);
        std::thread::sleep(std::time::Duration::from_millis(15));
        let end = tpass("0");
        assert_eq!(end.iter().map(|e| e.kind).collect::<Vec<_>>(), vec!["transitionend"]);
        assert_eq!(end[0].init(), "{propertyName:\"opacity\",elapsedTime:0.01,pseudoElement:\"\"}");
    }
}
