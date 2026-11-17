// Simple document and element properties pages use at startup (BBC's bootstrap script, 2026-11:
// document.referrer.split(...)). Each is only added when the document doesn't have it yet.
(function () {
  "use strict";
  const g = globalThis;
  const d = g.document;
  if (!d) return;
  const pad = (n) => String(n).padStart(2, "0");
  const loaded = new Date();
  const props = {
    // No referrer is passed between pages yet: "" is what browsers report then.
    referrer: { get() { return ""; } },
    // When there's no Last-Modified header, browsers report the time the page loaded.
    lastModified: { get() { const t = loaded; return `${pad(t.getMonth() + 1)}/${pad(t.getDate())}/${t.getFullYear()} ${pad(t.getHours())}:${pad(t.getMinutes())}:${pad(t.getSeconds())}`; } },
    visibilityState: { get() { return "visible"; } },
    hidden: { get() { return false; } },
    domain: { get() { try { return g.location.hostname; } catch (e) { return ""; } }, set() {} },
    dir: {
      get() { const v = (d.documentElement && d.documentElement.getAttribute("dir")) || ""; return /^(ltr|rtl|auto)$/i.test(v) ? v.toLowerCase() : ""; },
      set(v) { if (d.documentElement) d.documentElement.setAttribute("dir", v); },
    },
    designMode: { get() { return "off"; }, set() {} },
  };
  for (const [name, desc] of Object.entries(props)) {
    if (d[name] === undefined) Object.defineProperty(d, name, { ...desc, configurable: true, enumerable: true });
  }

  // Attributes scripts set as properties when they build <link>, <script> and <style> elements
  // (link.rel = "stylesheet"; script.async = true). Added only where nothing defines them.
  const proto = g.Element && g.Element.prototype;
  if (!proto) return;
  const strings = { rel: "rel", media: "media", integrity: "integrity", as: "as", charset: "charset", hreflang: "hreflang", referrerPolicy: "referrerpolicy", sizes: "sizes", imageSrcset: "imagesrcset", imageSizes: "imagesizes", fetchPriority: "fetchpriority" };
  for (const [prop, attr] of Object.entries(strings)) {
    if (prop in proto) continue;
    Object.defineProperty(proto, prop, { get() { return this.getAttribute(attr) || ""; }, set(v) { this.setAttribute(attr, String(v)); }, configurable: true });
  }
  if (!("crossOrigin" in proto)) {
    Object.defineProperty(proto, "crossOrigin", {
      get() { const v = this.getAttribute("crossorigin"); return v === null ? null : v.toLowerCase() === "use-credentials" ? "use-credentials" : "anonymous"; },
      set(v) { if (v === null) this.removeAttribute("crossorigin"); else this.setAttribute("crossorigin", String(v)); },
      configurable: true,
    });
  }
  for (const [prop, attr] of Object.entries({ async: "async", defer: "defer", noModule: "nomodule" })) {
    if (prop in proto) continue;
    Object.defineProperty(proto, prop, { get() { return this.hasAttribute(attr); }, set(v) { if (v) this.setAttribute(attr, ""); else this.removeAttribute(attr); }, configurable: true });
  }
  if (!("relList" in proto)) {
    Object.defineProperty(proto, "relList", {
      get() {
        const el = this;
        const list = () => (el.getAttribute("rel") || "").split(/\s+/).filter(Boolean);
        return {
          get length() { return list().length; },
          contains: (t) => list().includes(t),
          add: (...t) => el.setAttribute("rel", [...new Set([...list(), ...t])].join(" ")),
          remove: (...t) => el.setAttribute("rel", list().filter((x) => !t.includes(x)).join(" ")),
          supports: (t) => ["stylesheet", "preload", "modulepreload", "prefetch", "preconnect", "dns-prefetch", "icon", "alternate", "canonical", "manifest"].includes(String(t).toLowerCase()),
          toString: () => el.getAttribute("rel") || "",
          [Symbol.iterator]: () => list()[Symbol.iterator](),
        };
      },
      configurable: true,
    });
  }
})();
