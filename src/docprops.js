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

  // The navigation timing entry (performance.getEntriesByType("navigation")[0]): WordPress's
  // Interactivity API reads domContentLoadedEventStart to know whether the page is ready. Its
  // times fill in as the document's events fire (0 until then, as in other browsers).
  const perf = g.performance;
  if (perf && typeof perf.getEntriesByType === "function" && !perf.getEntriesByType("navigation").length) {
    const nav = {
      name: (g.location && g.location.href) || "", entryType: "navigation", startTime: 0, duration: 0,
      initiatorType: "navigation", type: "navigate", redirectCount: 0, nextHopProtocol: "h2",
      fetchStart: 0, domainLookupStart: 0, domainLookupEnd: 0, connectStart: 0, connectEnd: 0, requestStart: 0,
      responseStart: 0, responseEnd: 0, domInteractive: 0, domContentLoadedEventStart: 0, domContentLoadedEventEnd: 0,
      domComplete: 0, loadEventStart: 0, loadEventEnd: 0, transferSize: 0, encodedBodySize: 0, decodedBodySize: 0,
      toJSON() { return { ...this }; },
    };
    const now = () => Math.max(perf.now(), 0.1); // events always come after the navigation starts
    d.addEventListener("DOMContentLoaded", () => { nav.domInteractive = nav.domContentLoadedEventStart = now(); setTimeout(() => { nav.domContentLoadedEventEnd = now(); }, 0); }, { once: true });
    g.addEventListener("load", () => { nav.domComplete = nav.loadEventStart = now(); setTimeout(() => { nav.loadEventEnd = nav.duration = now(); }, 0); }, { once: true });
    const byType = perf.getEntriesByType.bind(perf), all = perf.getEntries ? perf.getEntries.bind(perf) : () => [], byName = perf.getEntriesByName ? perf.getEntriesByName.bind(perf) : () => [];
    perf.getEntriesByType = (t) => (String(t) === "navigation" ? [nav] : byType(t));
    perf.getEntries = () => [nav, ...all()];
    perf.getEntriesByName = (n, t) => (String(n) === nav.name && (t === undefined || t === "navigation") ? [nav] : []).concat(t === "navigation" ? [] : byName(n, t));
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
  // .text: a <script>'s or <title>'s source, a link's text, an option's text with spaces
  // collapsed (WordPress reads its settings with JSON.parse(script.text)).
  if (!("text" in proto)) {
    Object.defineProperty(proto, "text", {
      get() {
        const t = this.textContent;
        return (this.localName || "").toLowerCase() === "option" ? t.replace(/[\t\n\f\r ]+/g, " ").trim() : t;
      },
      set(v) { this.textContent = String(v); },
      configurable: true,
    });
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
