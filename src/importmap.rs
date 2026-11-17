//! Import maps: `<script type="importmap">`, so bare specifiers like `import "react"` resolve.
//!
//! Follows the HTML standard's algorithms: "parse an import map string", "sort and normalize a
//! module specifier map", "resolve a module specifier" and "merge existing and new import maps"
//! (a page may have several import maps; later ones only add entries that don't clash with
//! earlier ones or with specifiers already resolved).
//!
//! The parsed map lives in the script engine's context data, so both the module loader and
//! `import.meta.resolve()` see the same one.

use boa_engine::{js_string, Context, JsString, JsValue};
use std::cell::RefCell;

/// A specifier map: (normalized key, address or None if the entry was invalid), sorted so
/// longer keys come before keys that are their prefixes (descending code-unit order).
pub type SpecifierMap = Vec<(String, Option<String>)>;

#[derive(Default, Debug, Clone, PartialEq)]
pub struct ImportMap {
    pub imports: SpecifierMap,
    /// (scope prefix URL, its specifier map), sorted like a specifier map.
    pub scopes: Vec<(String, SpecifierMap)>,
    /// URL -> integrity metadata (kept for completeness; not enforced yet).
    pub integrity: Vec<(String, String)>,
}

/// The page's import map, plus the specifiers already resolved (later maps can't change those).
#[derive(Default)]
pub struct State {
    pub map: ImportMap,
    /// (referring script's URL, specifier as normalized, whether it can match a prefix key).
    resolved: Vec<(String, String, bool)>,
}

/// A JSON value, as much as an import map needs.
#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Obj(Vec<(String, J)>),
    Str(String),
    /// Anything else (number, array, null, boolean): the type name, for messages.
    Other(&'static str),
}

/// An error that stops an import map from being registered: (error kind, message).
#[derive(Debug, PartialEq)]
pub struct MapError(pub &'static str, pub String);

const SPECIAL: [&str; 6] = ["ftp", "file", "http", "https", "ws", "wss"];

fn parse_url(s: &str, base: Option<&str>) -> Option<url::Url> {
    match base {
        Some(b) => url::Url::parse(b).ok()?.join(s).ok(),
        None => url::Url::parse(s).ok(),
    }
}

/// "Resolve a URL-like module specifier": paths starting with `/`, `./` or `../` against the
/// base, otherwise only absolute URLs.
pub fn url_like(spec: &str, base: &str) -> Option<url::Url> {
    if spec.starts_with('/') || spec.starts_with("./") || spec.starts_with("../") {
        parse_url(spec, Some(base))
    } else {
        parse_url(spec, None)
    }
}

fn is_special(u: &url::Url) -> bool {
    SPECIAL.contains(&u.scheme())
}

fn warn(msg: &str) {
    log!("[importmap] {msg}");
}

/// "Sort and normalize a module specifier map".
fn normalize_map(entries: &[(String, J)], base: &str) -> SpecifierMap {
    let mut out: SpecifierMap = Vec::new();
    for (key, value) in entries {
        if key.is_empty() {
            warn("specifier keys may not be empty");
            continue;
        }
        let key = url_like(key, base).map(|u| u.to_string()).unwrap_or_else(|| key.clone());
        let address = match value {
            J::Str(v) => match url_like(v, base) {
                Some(a) if key.ends_with('/') && !a.as_str().ends_with('/') => {
                    warn(&format!("invalid address {a} for package specifier key \"{key}\": it must end with \"/\""));
                    None
                }
                Some(a) => Some(a.to_string()),
                None => {
                    warn(&format!("invalid address \"{v}\" for specifier key \"{key}\""));
                    None
                }
            },
            J::Obj(_) => {
                warn(&format!("invalid address for specifier key \"{key}\": addresses must be strings"));
                None
            }
            J::Other(_) => {
                warn(&format!("invalid address for specifier key \"{key}\": addresses must be strings"));
                None
            }
        };
        // Duplicate keys (after normalizing): the last one wins, as with JSON objects.
        out.retain(|(k, _)| *k != key);
        out.push((key, address));
    }
    out.sort_by(|a, b| code_units(&b.0).cmp(&code_units(&a.0)));
    out
}

fn code_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// "Parse an import map string" (after JSON parsing): `json` is the parsed top-level value.
pub fn parse(json: &J, base: &str) -> Result<ImportMap, MapError> {
    let J::Obj(top) = json else {
        return Err(MapError("TypeError", "an import map must be a JSON object".into()));
    };
    let mut map = ImportMap::default();
    for (key, value) in top {
        match key.as_str() {
            "imports" => match value {
                J::Obj(e) => map.imports = normalize_map(e, base),
                _ => return Err(MapError("TypeError", "the import map's \"imports\" must be a JSON object".into())),
            },
            "scopes" => match value {
                J::Obj(scopes) => {
                    for (prefix, m) in scopes {
                        let J::Obj(e) = m else {
                            return Err(MapError("TypeError", format!("the value of scope \"{prefix}\" must be a JSON object")));
                        };
                        let Some(prefix) = parse_url(prefix, Some(base)) else {
                            warn(&format!("invalid scope prefix URL \"{prefix}\""));
                            continue;
                        };
                        let prefix = prefix.to_string();
                        map.scopes.retain(|(p, _)| *p != prefix);
                        map.scopes.push((prefix, normalize_map(e, base)));
                    }
                    map.scopes.sort_by(|a, b| code_units(&b.0).cmp(&code_units(&a.0)));
                }
                _ => return Err(MapError("TypeError", "the import map's \"scopes\" must be a JSON object".into())),
            },
            "integrity" => match value {
                J::Obj(e) => {
                    for (k, v) in e {
                        let Some(u) = url_like(k, base) else {
                            warn(&format!("invalid integrity key \"{k}\""));
                            continue;
                        };
                        match v {
                            J::Str(s) => map.integrity.push((u.to_string(), s.clone())),
                            _ => warn(&format!("integrity metadata for \"{k}\" must be a string")),
                        }
                    }
                }
                _ => return Err(MapError("TypeError", "the import map's \"integrity\" must be a JSON object".into())),
            },
            other => warn(&format!("unknown top-level key \"{other}\" (only \"imports\", \"scopes\" and \"integrity\" are used)")),
        }
    }
    Ok(map)
}

/// "Resolve an imports match". Ok(None) = no entry matches; Err = an entry matches but is
/// blocked (null) or would escape its address.
fn imports_match(normalized: &str, as_url: Option<&url::Url>, map: &SpecifierMap) -> Result<Option<String>, String> {
    for (key, resolution) in map {
        if key == normalized {
            return match resolution {
                Some(r) => Ok(Some(r.clone())),
                None => Err(format!("resolution of \"{key}\" was blocked by a null entry in the import map")),
            };
        }
        if key.ends_with('/') && normalized.starts_with(key.as_str()) && as_url.is_none_or(is_special) {
            let Some(r) = resolution else {
                return Err(format!("resolution of \"{key}\" was blocked by a null entry in the import map"));
            };
            let after = &normalized[key.len()..];
            let Some(url) = parse_url(after, Some(r)) else {
                return Err(format!("resolution of \"{normalized}\" was blocked: \"{after}\" isn't a valid URL relative to {r}"));
            };
            if !url.as_str().starts_with(r.as_str()) {
                return Err(format!("resolution of \"{normalized}\" was blocked: it backtracks above its prefix \"{key}\""));
            }
            return Ok(Some(url.to_string()));
        }
    }
    Ok(None)
}

/// "Resolve a module specifier" against `map`, for a script at `base`.
pub fn resolve(map: &ImportMap, base: &str, spec: &str) -> Result<String, String> {
    let as_url = url_like(spec, base);
    let normalized = as_url.as_ref().map(|u| u.to_string()).unwrap_or_else(|| spec.to_string());
    for (prefix, scope) in &map.scopes {
        if prefix == base || (prefix.ends_with('/') && base.starts_with(prefix.as_str())) {
            if let Some(r) = imports_match(&normalized, as_url.as_ref(), scope)? {
                return Ok(r);
            }
        }
    }
    if let Some(r) = imports_match(&normalized, as_url.as_ref(), &map.imports)? {
        return Ok(r);
    }
    match as_url {
        Some(u) => Ok(u.to_string()),
        None => Err(format!(
            "Failed to resolve module specifier \"{spec}\": relative references must start with \"/\", \"./\" or \"../\", and no import map entry matched"
        )),
    }
}

/// Whether an entry with `key` would change how an already-resolved specifier resolves.
fn affects(key: &str, spec: &str, prefix_ok: bool) -> bool {
    key == spec || (prefix_ok && key.ends_with('/') && spec.starts_with(key))
}

/// "Merge existing and new import maps": entries already present, or that would change an
/// already-resolved specifier, are dropped from the new map.
pub fn merge(state: &mut State, new: ImportMap) {
    let resolved = &state.resolved;
    let old = &mut state.map;
    for (prefix, scope) in new.scopes {
        // Specifiers resolved by scripts inside this scope can't be remapped by it.
        let in_scope: Vec<&(String, String, bool)> = resolved
            .iter()
            .filter(|(referrer, _, _)| *referrer == prefix || (prefix.ends_with('/') && referrer.starts_with(prefix.as_str())))
            .collect();
        let scope: SpecifierMap = scope
            .into_iter()
            .filter(|(k, _)| {
                let keep = !in_scope.iter().any(|(_, s, p)| affects(k, s, *p));
                if !keep {
                    warn(&format!("ignored scoped entry \"{k}\" in {prefix}: that specifier was already resolved"));
                }
                keep
            })
            .collect();
        match old.scopes.iter_mut().find(|(p, _)| *p == prefix) {
            Some((_, existing)) => {
                for (k, v) in scope {
                    if existing.iter().any(|(ek, _)| *ek == k) {
                        warn(&format!("ignored scoped entry \"{k}\" in {prefix}: an earlier import map defines it"));
                    } else {
                        existing.push((k, v));
                    }
                }
                existing.sort_by(|a, b| code_units(&b.0).cmp(&code_units(&a.0)));
            }
            None => old.scopes.push((prefix, scope)),
        }
    }
    old.scopes.sort_by(|a, b| code_units(&b.0).cmp(&code_units(&a.0)));
    for (k, v) in new.imports {
        if resolved.iter().any(|(_, s, p)| affects(&k, s, *p)) {
            warn(&format!("ignored entry \"{k}\": that specifier was already resolved"));
        } else if old.imports.iter().any(|(ek, _)| *ek == k) {
            warn(&format!("ignored entry \"{k}\": an earlier import map defines it"));
        } else {
            old.imports.push((k, v));
        }
    }
    old.imports.sort_by(|a, b| code_units(&b.0).cmp(&code_units(&a.0)));
    for (k, v) in new.integrity {
        if !old.integrity.iter().any(|(ek, _)| *ek == k) {
            old.integrity.push((k, v));
        }
    }
}

// ---------------------------------------------------------------- the script engine's side

type Cell = RefCell<State>;

/// Resolve `spec` for a script at `base` with the page's import map, and remember it (so a
/// later import map can't change what it resolves to).
pub fn resolve_in(ctx: &Context, base: &str, spec: &str) -> Result<String, String> {
    let base = script_base(base);
    let Some(cell) = ctx.get_data::<Cell>() else {
        return resolve(&ImportMap::default(), base, spec);
    };
    let mut state = cell.borrow_mut();
    let r = resolve(&state.map, base, spec)?;
    let as_url = url_like(spec, base);
    let normalized = as_url.as_ref().map(|u| u.to_string()).unwrap_or_else(|| spec.to_string());
    let prefix_ok = as_url.as_ref().is_none_or(is_special);
    let record = (base.to_string(), normalized, prefix_ok);
    if !state.resolved.contains(&record) {
        state.resolved.push(record);
    }
    Ok(r)
}

/// Inline modules are kept as `page#inline-N` so each is its own module; for import maps
/// their base is the page itself.
fn script_base(url: &str) -> &str {
    match url.rfind("#inline-") {
        Some(i) => &url[..i],
        None => url,
    }
}

/// Turn a parsed JSON value into a `J` (objects and strings; the rest only by type).
fn to_j(v: &JsValue, ctx: &mut Context, depth: u32) -> J {
    if let Some(s) = v.as_string() {
        return J::Str(s.to_std_string_escaped());
    }
    let Some(obj) = v.as_object() else {
        return J::Other(if v.is_null() { "null" } else if v.is_boolean() { "boolean" } else { "number" });
    };
    if obj.is_array() {
        return J::Other("array");
    }
    if depth > 3 {
        return J::Obj(Vec::new());
    }
    let mut out = Vec::new();
    let keys = obj.own_property_keys(ctx).unwrap_or_default();
    for k in keys {
        let name = match &k {
            boa_engine::property::PropertyKey::String(s) => s.to_std_string_escaped(),
            boa_engine::property::PropertyKey::Index(i) => i.get().to_string(),
            boa_engine::property::PropertyKey::Symbol(_) => continue,
        };
        let val = obj.get(k, ctx).unwrap_or_default();
        out.push((name, to_j(&val, ctx, depth + 1)));
    }
    J::Obj(out)
}

/// Register an inline `<script type="importmap">`'s text for the page at `base`.
/// Err((kind, message)) when the map is rejected (bad JSON or wrong shape).
pub fn register(ctx: &mut Context, text: &str, base: &str) -> Result<(), MapError> {
    let parse_fn = ctx
        .global_object()
        .get(js_string!("JSON"), ctx)
        .ok()
        .and_then(|j| j.as_object().map(|o| (o.clone(), o.get(js_string!("parse"), ctx).ok())))
        .and_then(|(j, f)| f.and_then(|f| f.as_callable().map(|f| (j, f.clone()))));
    let Some((json_obj, parse_fn)) = parse_fn else {
        return Err(MapError("TypeError", "JSON.parse is missing".into()));
    };
    let value = parse_fn
        .call(&JsValue::from(json_obj), &[JsValue::from(JsString::from(text))], ctx)
        .map_err(|e| MapError("SyntaxError", format!("the import map isn't valid JSON: {e}")))?;
    let json = to_j(&value, ctx, 0);
    let map = parse(&json, base)?;
    if ctx.get_data::<Cell>().is_none() {
        ctx.insert_data(Cell::default());
    }
    let cell = ctx.get_data::<Cell>().expect("import map state");
    merge(&mut cell.borrow_mut(), map);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: &[(&str, J)]) -> J {
        J::Obj(v.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }
    fn s(v: &str) -> J {
        J::Str(v.into())
    }
    const BASE: &str = "https://example.com/app/index.html";

    #[test]
    fn bare_and_prefix_specifiers() {
        let m = parse(
            &obj(&[(
                "imports",
                obj(&[("react", s("/vendor/react.js")), ("lodash/", s("https://cdn.test/lodash/")), ("moment", s("./moment.js"))]),
            )]),
            BASE,
        )
        .unwrap();
        assert_eq!(resolve(&m, BASE, "react").unwrap(), "https://example.com/vendor/react.js");
        assert_eq!(resolve(&m, BASE, "moment").unwrap(), "https://example.com/app/moment.js");
        assert_eq!(resolve(&m, BASE, "lodash/map.js").unwrap(), "https://cdn.test/lodash/map.js");
        assert_eq!(resolve(&m, BASE, "./x.js").unwrap(), "https://example.com/app/x.js");
        assert!(resolve(&m, BASE, "vue").is_err());
    }

    #[test]
    fn url_keys_remap_urls() {
        let m = parse(&obj(&[("imports", obj(&[("/app/old.js", s("/app/new.js"))]))]), BASE).unwrap();
        assert_eq!(resolve(&m, BASE, "./old.js").unwrap(), "https://example.com/app/new.js");
        assert_eq!(resolve(&m, BASE, "https://example.com/app/old.js").unwrap(), "https://example.com/app/new.js");
    }

    #[test]
    fn longest_prefix_wins() {
        let m = parse(&obj(&[("imports", obj(&[("a/", s("/1/")), ("a/b/", s("/2/"))]))]), BASE).unwrap();
        assert_eq!(resolve(&m, BASE, "a/b/c.js").unwrap(), "https://example.com/2/c.js");
        assert_eq!(resolve(&m, BASE, "a/c.js").unwrap(), "https://example.com/1/c.js");
    }

    #[test]
    fn scopes() {
        let m = parse(
            &obj(&[
                ("imports", obj(&[("dep", s("/dep-v1.js"))])),
                ("scopes", obj(&[("/legacy/", obj(&[("dep", s("/dep-v0.js"))]))])),
            ]),
            BASE,
        )
        .unwrap();
        assert_eq!(resolve(&m, "https://example.com/legacy/a.js", "dep").unwrap(), "https://example.com/dep-v0.js");
        assert_eq!(resolve(&m, "https://example.com/new/a.js", "dep").unwrap(), "https://example.com/dep-v1.js");
    }

    #[test]
    fn invalid_entries_and_blocking() {
        let m = parse(
            &obj(&[(
                "imports",
                obj(&[("bad", J::Other("number")), ("pkg/", s("/no-slash")), ("", s("/x.js")), ("ok/", s("/ok/"))]),
            )]),
            BASE,
        )
        .unwrap();
        assert!(resolve(&m, BASE, "bad").unwrap_err().contains("blocked"));
        assert!(resolve(&m, BASE, "pkg/x").unwrap_err().contains("blocked"));
        // Backtracking out of a prefix is blocked.
        assert!(resolve(&m, BASE, "ok/../secret.js").unwrap_err().contains("backtracks"));
        assert_eq!(m.imports.len(), 3); // the empty key was dropped
    }

    #[test]
    fn shape_errors() {
        assert_eq!(parse(&J::Other("array"), BASE).unwrap_err().0, "TypeError");
        assert_eq!(parse(&obj(&[("imports", s("x"))]), BASE).unwrap_err().0, "TypeError");
        assert_eq!(parse(&obj(&[("scopes", obj(&[("/a/", s("x"))]))]), BASE).unwrap_err().0, "TypeError");
        assert!(parse(&obj(&[("other", s("x"))]), BASE).is_ok());
    }

    #[test]
    fn merging_keeps_first_and_resolved() {
        let mut st = State::default();
        merge(&mut st, parse(&obj(&[("imports", obj(&[("a", s("/a1.js"))]))]), BASE).unwrap());
        st.resolved.push((BASE.to_string(), "b".to_string(), true));
        merge(&mut st, parse(&obj(&[("imports", obj(&[("a", s("/a2.js")), ("b", s("/b.js")), ("c", s("/c.js"))]))]), BASE).unwrap());
        assert_eq!(resolve(&st.map, BASE, "a").unwrap(), "https://example.com/a1.js");
        assert!(resolve(&st.map, BASE, "b").is_err());
        assert_eq!(resolve(&st.map, BASE, "c").unwrap(), "https://example.com/c.js");
    }

    #[test]
    fn register_from_json_text() {
        let mut ctx = Context::default();
        register(&mut ctx, r#"{"imports": {"react": "/r.js"}, "scopes": {"/s/": {"react": "/r2.js"}}}"#, BASE).unwrap();
        assert_eq!(resolve_in(&ctx, BASE, "react").unwrap(), "https://example.com/r.js");
        assert_eq!(resolve_in(&ctx, "https://example.com/s/m.js", "react").unwrap(), "https://example.com/r2.js");
        assert_eq!(resolve_in(&ctx, "https://example.com/app/index.html#inline-4", "react").unwrap(), "https://example.com/r.js");
        assert_eq!(register(&mut ctx, "{not json", BASE).unwrap_err().0, "SyntaxError");
        assert_eq!(register(&mut ctx, "[]", BASE).unwrap_err().0, "TypeError");
    }
}
