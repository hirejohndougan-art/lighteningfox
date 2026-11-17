//! ES modules: `<script type=module>`, `import` and `import()`.
//!
//! Boa parses, links and runs modules; this loader finds them. A specifier is resolved
//! against the importing module's URL (kept as the module's "path"), or the page's URL for
//! classic scripts and inline modules. Each URL is fetched and parsed once per page, so
//! every import of the same URL gets the same module, as the spec requires.

use crate::net;
use boa_engine::module::{ModuleLoader, Referrer};
use boa_engine::{Context, JsNativeError, JsResult, JsString, Module, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

#[derive(Default)]
pub struct Loader {
    /// Modules already loaded, by absolute URL.
    cache: RefCell<HashMap<String, Module>>,
}

/// Resolve a module specifier. Like browsers (without import maps), only URLs and paths
/// starting with `/`, `./` or `../` are allowed; bare names like `"react"` are an error.
pub fn resolve_specifier(base: &str, spec: &str) -> Option<String> {
    let has_scheme = spec.split_once(':').is_some_and(|(s, _)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)));
    if has_scheme || spec.starts_with('/') || spec.starts_with("./") || spec.starts_with("../") {
        Some(net::resolve(base, spec))
    } else {
        None
    }
}

/// The URL a module was loaded from, as kept in its path.
pub fn module_url(path: Option<&Path>) -> Option<String> {
    path.map(|p| p.to_string_lossy().into_owned())
}

impl Loader {
    /// Parse a module's source text, remembering `url` as where it came from.
    pub fn parse(&self, code: &str, url: &str, ctx: &mut Context) -> JsResult<Module> {
        let path = Path::new(url);
        Module::parse(Source::from_bytes(code.as_bytes()).with_path(path), None, ctx)
    }

    fn fetch(&self, url: &str, page: &str, json: bool, ctx: &mut Context) -> JsResult<Module> {
        if let Some(m) = self.cache.borrow().get(url) {
            return Ok(m.clone());
        }
        let (_, body) = net::fetch_sub(url, page, crate::security::Dest::Script)
            .map_err(|e| JsNativeError::typ().with_message(format!("couldn't load module {url}: {e}")))?;
        let module = if json { Module::parse_json(JsString::from(body.as_str()), ctx)? } else { self.parse(&body, url, ctx)? };
        self.cache.borrow_mut().insert(url.to_string(), module.clone());
        Ok(module)
    }
}

impl ModuleLoader for Loader {
    async fn load_imported_module(
        self: Rc<Self>,
        referrer: Referrer,
        request: boa_engine::module::ModuleRequest,
        context: &RefCell<&mut Context>,
    ) -> JsResult<Module> {
        let ctx = &mut *context.borrow_mut();
        let page = crate::js::page_url(ctx);
        let base = module_url(referrer.path()).unwrap_or_else(|| page.clone());
        let spec = request.specifier().to_std_string_escaped();
        let Some(url) = resolve_specifier(&base, &spec) else {
            return Err(JsNativeError::typ()
                .with_message(format!("Failed to resolve module specifier \"{spec}\": relative references must start with \"/\", \"./\" or \"../\""))
                .into());
        };
        if let Some(policy) = crate::security::policy_for(&page) {
            if !policy.csp.allows_url(crate::security::Dest::Script, &url, None, true) {
                return Err(JsNativeError::typ().with_message(format!("module {url} blocked by Content Security Policy")).into());
            }
        }
        let json = request.get_attribute("type").is_some_and(|t| t.to_std_string_escaped() == "json");
        self.fetch(&url, &page, json, ctx)
    }

    fn init_import_meta(self: Rc<Self>, import_meta: &boa_engine::JsObject, module: &Module, context: &mut Context) {
        let url = module_url(module.path()).unwrap_or_else(|| crate::js::page_url(context));
        let url = url.split('#').next().unwrap_or("").to_string();
        let _ = import_meta.set(boa_engine::js_string!("url"), JsString::from(url.as_str()), false, context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specifiers() {
        let base = "https://example.com/js/app.js";
        assert_eq!(resolve_specifier(base, "./a.js").as_deref(), Some("https://example.com/js/a.js"));
        assert_eq!(resolve_specifier(base, "../b.js").as_deref(), Some("https://example.com/b.js"));
        assert_eq!(resolve_specifier(base, "/c.js").as_deref(), Some("https://example.com/c.js"));
        assert_eq!(resolve_specifier(base, "https://cdn.test/x.js").as_deref(), Some("https://cdn.test/x.js"));
        assert_eq!(resolve_specifier(base, "react"), None);
    }
}
