//! ES modules: `<script type=module>`, `import` and `import()`.
//!
//! Boa parses, links and runs modules; this loader finds them. A specifier is resolved
//! against the importing module's URL (kept as the module's "path"), or the page's URL for
//! classic scripts and inline modules, after the page's import map (`importmap.rs`) has had
//! its say. Each URL is fetched and parsed once per page, so
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
        // Through the page's import map (if any), then as a URL or path.
        let url = crate::importmap::resolve_in(ctx, &base, &spec).map_err(|m| JsNativeError::typ().with_message(m))?;
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
        // import.meta.resolve(specifier): what an import of it from this module would load.
        let resolve = boa_engine::NativeFunction::from_copy_closure_with_captures(
            |_, args, base: &JsString, ctx| {
                let spec = args.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped();
                crate::importmap::resolve_in(ctx, &base.to_std_string_escaped(), &spec)
                    .map(|u| boa_engine::JsValue::from(JsString::from(u.as_str())))
                    .map_err(|m| JsNativeError::typ().with_message(m).into())
            },
            JsString::from(module_url(module.path()).unwrap_or_else(|| crate::js::page_url(context)).as_str()),
        );
        let f = boa_engine::object::FunctionObjectBuilder::new(context.realm(), resolve).name(boa_engine::js_string!("resolve")).length(1).build();
        let _ = import_meta.set(boa_engine::js_string!("resolve"), f, false, context);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn specifiers() {
        let resolve_specifier = |base: &str, spec: &str| crate::importmap::resolve(&Default::default(), base, spec).ok();
        let base = "https://example.com/js/app.js";
        assert_eq!(resolve_specifier(base, "./a.js").as_deref(), Some("https://example.com/js/a.js"));
        assert_eq!(resolve_specifier(base, "../b.js").as_deref(), Some("https://example.com/b.js"));
        assert_eq!(resolve_specifier(base, "/c.js").as_deref(), Some("https://example.com/c.js"));
        assert_eq!(resolve_specifier(base, "https://cdn.test/x.js").as_deref(), Some("https://cdn.test/x.js"));
        assert_eq!(resolve_specifier(base, "react"), None);
    }
}
