//! The browser's own language: which locale the UI is shown in (and what pages see as
//! `navigator.language` and the default for `Intl`), and its translated strings, kept as
//! Fluent files in `locales/<lang>/browser.ftl`.

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use std::sync::{OnceLock, RwLock};
use unic_langid::LanguageIdentifier;

/// (code, the language's own name, Fluent source). English is the fallback for missing strings.
pub const LANGUAGES: &[(&str, &str, &str)] = &[
    ("en-US", "English", include_str!("../locales/en-US/browser.ftl")),
    ("es", "Español", include_str!("../locales/es/browser.ftl")),
    ("fr", "Français", include_str!("../locales/fr/browser.ftl")),
    ("de", "Deutsch", include_str!("../locales/de/browser.ftl")),
    ("ar", "العربية", include_str!("../locales/ar/browser.ftl")),
    ("he", "עברית", include_str!("../locales/he/browser.ftl")),
];

/// Languages written right to left (the UI is mirrored for them).
const RTL: &[&str] = &["ar", "he", "fa", "ur", "yi", "ps", "sd", "ug", "dv", "ckb"];

/// Turn a POSIX locale ("de_DE.UTF-8", "C") into a BCP 47 tag ("de-DE", none).
pub fn from_posix(v: &str) -> Option<String> {
    let v = v.split(['.', '@']).next()?.trim();
    if v.is_empty() || v == "C" || v == "POSIX" {
        return None;
    }
    Some(v.replace('_', "-"))
}

/// The desktop's language, from the usual environment variables.
fn system_locale() -> Option<String> {
    if let Ok(l) = std::env::var("LANGUAGE") {
        if let Some(first) = l.split(':').find_map(from_posix) {
            return Some(first);
        }
    }
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().and_then(|v| from_posix(&v)))
}

fn current() -> &'static RwLock<String> {
    static C: OnceLock<RwLock<String>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(choose(&crate::settings::get().language)))
}

/// The locale for a language setting: "auto" follows the desktop, else the code chosen.
fn choose(setting: &str) -> String {
    match setting.trim() {
        "" | "auto" => system_locale().unwrap_or_else(|| "en-US".into()),
        s => s.to_string(),
    }
}

/// The UI locale, as a BCP 47 tag ("en-US", "de-DE"…).
pub fn locale() -> String {
    current().read().map(|s| s.clone()).unwrap_or_else(|_| "en-US".into())
}

/// Called when the language setting changes.
pub fn set_language(setting: &str) {
    if let Ok(mut w) = current().write() {
        *w = choose(setting);
    }
}

/// Make the chosen language the process's default, so Boa's `Intl` uses it for pages. Call
/// once at startup, before any threads start.
pub fn apply_to_process() {
    let loc = locale();
    // Safe here: called from main before other threads exist.
    unsafe { std::env::set_var("LANGUAGE", loc.replace('-', "_")) };
}

/// Whether the UI reads right to left.
pub fn rtl() -> bool {
    is_rtl(&locale())
}

pub fn is_rtl(loc: &str) -> bool {
    let lang = loc.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    RTL.contains(&lang.as_str())
}

fn bundle_for(code: &str, src: &str) -> FluentBundle<FluentResource> {
    let id: LanguageIdentifier = code.parse().unwrap_or_default();
    let mut b = FluentBundle::new_concurrent(vec![id]);
    // Directional isolation marks around placeables would show up as boxes in our UI font.
    b.set_use_isolating(false);
    match FluentResource::try_new(src.to_string()) {
        Ok(r) => {
            let _ = b.add_resource(r);
        }
        Err((r, errs)) => {
            log!("[i18n] {code}: {} syntax errors in browser.ftl", errs.len());
            let _ = b.add_resource(r);
        }
    }
    b
}

fn bundles() -> &'static Vec<(String, FluentBundle<FluentResource>)> {
    static B: OnceLock<Vec<(String, FluentBundle<FluentResource>)>> = OnceLock::new();
    B.get_or_init(|| LANGUAGES.iter().map(|(c, _, src)| (c.to_string(), bundle_for(c, src))).collect())
}

/// The best bundle for a locale: an exact match, else the same language, else English.
fn pick(loc: &str) -> usize {
    let b = bundles();
    let lang = loc.split('-').next().unwrap_or("").to_ascii_lowercase();
    b.iter()
        .position(|(c, _)| c.eq_ignore_ascii_case(loc))
        .or_else(|| b.iter().position(|(c, _)| c.split('-').next().is_some_and(|l| l.eq_ignore_ascii_case(&lang))))
        .unwrap_or(0)
}

/// Look up a message in a given locale, with arguments. Falls back to English, then the id.
pub fn translate(loc: &str, id: &str, args: &[(&str, &str)]) -> String {
    let b = bundles();
    let fa = if args.is_empty() {
        None
    } else {
        let mut a = FluentArgs::new();
        for (k, v) in args {
            // Numbers stay numbers, so plural rules ({ $n -> [one] … }) work.
            match v.parse::<f64>() {
                Ok(n) => a.set(*k, n),
                Err(_) => a.set(*k, v.to_string()),
            }
        }
        Some(a)
    };
    for i in [pick(loc), 0] {
        let bundle = &b[i].1;
        if let Some(pattern) = bundle.get_message(id).and_then(|m| m.value()) {
            let mut errs = Vec::new();
            return bundle.format_pattern(pattern, fa.as_ref(), &mut errs).into_owned();
        }
    }
    id.to_string()
}

/// A UI string in the current language.
pub fn t(id: &str) -> String {
    translate(&locale(), id, &[])
}

/// A UI string with arguments: `tf("tabs-count", &[("n", "3")])`.
pub fn tf(id: &str, args: &[(&str, &str)]) -> String {
    translate(&locale(), id, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_locales() {
        assert_eq!(from_posix("de_DE.UTF-8").as_deref(), Some("de-DE"));
        assert_eq!(from_posix("C"), None);
        assert_eq!(from_posix("sr_RS@latin").as_deref(), Some("sr-RS"));
    }

    #[test]
    fn lookups_and_fallback() {
        assert_eq!(translate("en-US", "menu-reload", &[]), "Reload");
        assert_eq!(translate("es-MX", "menu-reload", &[]), "Volver a cargar");
        assert_eq!(translate("de-AT", "menu-back", &[]), "Zurück");
        // Unknown language: English.
        assert_eq!(translate("xx", "menu-reload", &[]), "Reload");
        // Unknown id: the id itself.
        assert_eq!(translate("fr", "no-such-string", &[]), "no-such-string");
    }

    #[test]
    fn arguments_and_plurals() {
        assert_eq!(translate("en-US", "toast-printed", &[("n", "1")]), "Sent 1 page to the printer");
        assert_eq!(translate("en-US", "toast-printed", &[("n", "3")]), "Sent 3 pages to the printer");
        assert_eq!(translate("fr", "toast-zoom", &[("pct", "150")]), "Zoom 150 %  ·  Ctrl+0 pour réinitialiser");
    }

    #[test]
    fn every_language_has_every_english_string() {
        let en = LANGUAGES[0].2;
        let ids: Vec<&str> = en.lines().filter_map(|l| l.split_once(" =").map(|p| p.0)).filter(|id| !id.starts_with('#') && !id.starts_with(' ') && !id.is_empty()).collect();
        assert!(ids.len() > 20);
        for (code, _, src) in &LANGUAGES[1..] {
            for id in &ids {
                assert!(src.lines().any(|l| l.starts_with(&format!("{id} ="))), "{code} lacks {id}");
            }
        }
    }

    #[test]
    fn rtl_languages() {
        assert!(is_rtl("ar-EG") && is_rtl("he") && !is_rtl("en-US") && !is_rtl("fr"));
    }
}
