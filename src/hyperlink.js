// <a> and <area> as URLs (HTMLHyperlinkElementUtils): href resolved against the document's base
// URL, and protocol, host, hostname, port, pathname, search, hash, origin, username, password,
// each readable and settable. From the New York Times (2026-11): its Sentry/axios code parses
// URLs by setting an <a>'s href and reading a.protocol.replace(…), which threw on undefined.
// <link>'s href is resolved too. Also document.baseURI when nothing defines it yet.
(function () {
  "use strict";
  const g = globalThis;
  const d = g.document;
  const E = g.Element && g.Element.prototype;
  if (!d || !E || typeof g.URL !== "function") return;

  const baseURL = () => {
    const b = d.querySelector && d.querySelector("base[href]");
    const page = g.location ? g.location.href : "";
    if (b) { try { return new URL(b.getAttribute("href"), page).href; } catch (e) { /* fall through */ } }
    return page;
  };
  if (!("baseURI" in d)) Object.defineProperty(d, "baseURI", { get: baseURL, configurable: true, enumerable: true });

  const isLink = (el) => { const n = el.localName; return n === "a" || n === "area"; };
  const hrefOwners = new Set(["a", "area", "link", "base"]);
  // The element's URL, or null when it has no href or the href doesn't parse.
  const urlOf = (el) => {
    const h = el.getAttribute("href");
    if (h === null) return null;
    try { return new URL(h, el.localName === "base" ? (g.location ? g.location.href : "") : baseURL()); } catch (e) { return null; }
  };

  const oldHref = Object.getOwnPropertyDescriptor(E, "href");
  Object.defineProperty(E, "href", {
    get() {
      if (!hrefOwners.has(this.localName)) return oldHref && oldHref.get ? oldHref.get.call(this) : this.getAttribute("href") || "";
      const h = this.getAttribute("href");
      if (h === null) return "";
      const u = urlOf(this);
      return u ? u.href : h;
    },
    set(v) { if (oldHref && oldHref.set) oldHref.set.call(this, v); else this.setAttribute("href", String(v)); },
    configurable: true,
  });

  // Each part: what it reads as with no (or a bad) URL, and how to set it on a URL object.
  const parts = {
    protocol: [":", (u, v) => { const p = String(v).replace(/:.*$/, ""); if (/^[a-z][a-z0-9+.-]*$/i.test(p)) u.protocol = p.toLowerCase() + ":"; }],
    username: ["", (u, v) => { u.username = String(v); }],
    password: ["", (u, v) => { u.password = String(v); }],
    host: ["", (u, v) => { const m = /^([^:\/?#]*)(?::(\d*))?/.exec(String(v)); if (m && m[1]) { u.hostname = m[1].toLowerCase(); if (m[2] !== undefined) u.port = m[2]; } }],
    hostname: ["", (u, v) => { const h = String(v).replace(/[:\/?#].*$/, ""); if (h) u.hostname = h.toLowerCase(); }],
    port: ["", (u, v) => { u.port = String(v).replace(/\D.*$/, ""); }],
    pathname: ["", (u, v) => { const p = String(v); u.pathname = p.startsWith("/") ? p : "/" + p; }],
    search: ["", (u, v) => { u.search = String(v); }],
    hash: ["", (u, v) => { const h = String(v).replace(/^#/, ""); u.hash = h ? "#" + h : ""; }],
  };
  for (const [name, [empty, set]] of Object.entries(parts)) {
    if (Object.prototype.hasOwnProperty.call(E, name)) continue;
    Object.defineProperty(E, name, {
      get() {
        if (!isLink(this)) return undefined;
        const u = urlOf(this);
        return u ? u[name] : empty;
      },
      set(v) {
        if (!isLink(this)) return;
        const u = urlOf(this);
        if (!u) return;
        set(u, v);
        this.setAttribute("href", u.href);
      },
      configurable: true,
    });
  }
  if (!Object.prototype.hasOwnProperty.call(E, "origin")) {
    Object.defineProperty(E, "origin", {
      get() {
        if (!isLink(this)) return undefined;
        const u = urlOf(this);
        return u ? u.origin : "";
      },
      configurable: true,
    });
  }
})();
