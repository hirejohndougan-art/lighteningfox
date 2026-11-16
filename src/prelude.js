// LighteningFox DOM bindings.
// The Rust side exposes a handful of primitives (__tag, __children, __append, ...)
// that work on node ids. Everything web pages see (document, Element, events,
// timers...) is built here in JavaScript on top of them.
(function () {
  "use strict";
  const g = globalThis;
  const cache = new Map();

  function wrap(id) {
    if (id === undefined || id === null || id < 0) return null;
    let n = cache.get(id);
    if (n) return n;
    const t = __type(id);
    n = t === 9 ? document : t === 3 ? new Text(id) : t === 8 ? new Comment(id) : new Element(id);
    cache.set(id, n);
    return n;
  }
  const idOf = (n) => (n && n.__id !== undefined ? n.__id : -1);
  const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());

  // ------------------------------------------------------------ events
  class Event {
    constructor(type, init = {}) {
      this.type = type;
      this.bubbles = !!init.bubbles;
      this.cancelable = !!init.cancelable;
      this.defaultPrevented = false;
      this.target = null;
      this.currentTarget = null;
      this.timeStamp = __nowMs;
      this._stop = false;
    }
    preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
    stopPropagation() { this._stop = true; }
    stopImmediatePropagation() { this._stop = true; }
  }
  class CustomEvent extends Event {
    constructor(type, init = {}) { super(type, init); this.detail = init.detail ?? null; }
  }
  class EventTarget {
    addEventListener(type, fn, opts) {
      if (!fn) return;
      const l = (this.__listeners ||= {});
      (l[type] ||= []).push({ fn, once: !!(opts && opts.once) });
    }
    removeEventListener(type, fn) {
      const l = this.__listeners && this.__listeners[type];
      if (l) this.__listeners[type] = l.filter((x) => x.fn !== fn);
    }
    __fire(ev) {
      ev.currentTarget = this;
      const list = (this.__listeners && this.__listeners[ev.type]) || [];
      for (const entry of list.slice()) {
        if (entry.once) this.removeEventListener(ev.type, entry.fn);
        try {
          typeof entry.fn === "function" ? entry.fn.call(this, ev) : entry.fn.handleEvent(ev);
        } catch (e) { console.error(String(e)); }
      }
      const prop = this["on" + ev.type];
      if (typeof prop === "function") {
        try { if (prop.call(this, ev) === false) ev.preventDefault(); } catch (e) { console.error(String(e)); }
      } else if (this.getAttribute && this.getAttribute("on" + ev.type)) {
        try {
          const f = new Function("event", this.getAttribute("on" + ev.type));
          if (f.call(this, ev) === false) ev.preventDefault();
        } catch (e) { console.error(String(e)); }
      }
    }
    dispatchEvent(ev) {
      ev.target = this;
      let node = this;
      while (node) {
        node.__fire(ev);
        if (!ev.bubbles || ev._stop) break;
        node = node === document ? g : node.parentNode;
        if (node === g) { EventTarget.prototype.__fire.call(g, ev); break; }
      }
      return !ev.defaultPrevented;
    }
  }

  // ------------------------------------------------------------ nodes
  class Node extends EventTarget {
    constructor(id) { super(); this.__id = id; }
    get nodeType() { return __type(this.__id); }
    get ownerDocument() { return document; }
    get parentNode() { return wrap(__parent(this.__id)); }
    get parentElement() { const p = this.parentNode; return p && p.nodeType === 1 ? p : null; }
    get childNodes() { return __children(this.__id).map(wrap); }
    get firstChild() { return wrap(__children(this.__id)[0]); }
    get lastChild() { const c = __children(this.__id); return wrap(c[c.length - 1]); }
    get nextSibling() { return this.__sibling(1); }
    get previousSibling() { return this.__sibling(-1); }
    __sibling(d) {
      const p = __parent(this.__id);
      if (p < 0) return null;
      const c = __children(p);
      return wrap(c[c.indexOf(this.__id) + d]);
    }
    hasChildNodes() { return __children(this.__id).length > 0; }
    get isConnected() { let n = this.__id; while (n > 0) n = __parent(n); return n === 0; }
    get textContent() { return __text(this.__id); }
    set textContent(v) { __setText(this.__id, v == null ? "" : String(v)); }
    get nodeValue() { return this.nodeType === 1 ? null : this.textContent; }
    set nodeValue(v) { if (this.nodeType !== 1) this.textContent = v; }
    appendChild(c) {
      if (c instanceof DocumentFragment) { for (const k of c.childNodes) this.appendChild(k); return c; }
      __insert(this.__id, idOf(c), -1); return c;
    }
    insertBefore(c, ref) {
      if (c instanceof DocumentFragment) { for (const k of c.childNodes) this.insertBefore(k, ref); return c; }
      __insert(this.__id, idOf(c), idOf(ref)); return c;
    }
    removeChild(c) { __detach(idOf(c)); return c; }
    replaceChild(n, o) { this.insertBefore(n, o); __detach(idOf(o)); return o; }
    contains(o) { let n = idOf(o); while (n >= 0) { if (n === this.__id) return true; n = __parent(n); } return false; }
    cloneNode(deep) { return wrap(__clone(this.__id, !!deep)); }
    get __isNode() { return true; }
  }
  class CharacterData extends Node {
    get data() { return this.textContent; }
    set data(v) { this.textContent = v; }
    get length() { return this.textContent.length; }
    remove() { __detach(this.__id); }
  }
  class Text extends CharacterData { get nodeName() { return "#text"; } }
  class Comment extends CharacterData { get nodeName() { return "#comment"; } }

  function nodesFrom(args) {
    return args.map((a) => (typeof a === "string" ? document.createTextNode(a) : a));
  }

  class Element extends Node {
    get tagName() { return __tag(this.__id).toUpperCase(); }
    get nodeName() { return this.tagName; }
    get localName() { return __tag(this.__id); }
    getAttribute(n) { return __getAttr(this.__id, String(n).toLowerCase()); }
    setAttribute(n, v) { __setAttr(this.__id, String(n).toLowerCase(), String(v)); }
    removeAttribute(n) { __removeAttr(this.__id, String(n).toLowerCase()); }
    hasAttribute(n) { return this.getAttribute(n) !== null; }
    toggleAttribute(n, force) {
      const on = force === undefined ? !this.hasAttribute(n) : force;
      on ? this.setAttribute(n, "") : this.removeAttribute(n);
      return on;
    }
    getAttributeNames() { return __attrNames(this.__id); }
    get attributes() { return this.getAttributeNames().map((name) => ({ name, value: this.getAttribute(name) })); }
    get id() { return this.getAttribute("id") || ""; }
    set id(v) { this.setAttribute("id", v); }
    get className() { return this.getAttribute("class") || ""; }
    set className(v) { this.setAttribute("class", v); }
    get classList() {
      const el = this;
      const get = () => el.className.split(/\s+/).filter(Boolean);
      const set = (a) => { el.className = a.join(" "); };
      return {
        add(...c) { const a = get(); for (const x of c) if (!a.includes(x)) a.push(x); set(a); },
        remove(...c) { set(get().filter((x) => !c.includes(x))); },
        toggle(c, force) {
          const has = get().includes(c);
          const on = force === undefined ? !has : force;
          on ? this.add(c) : this.remove(c);
          return on;
        },
        contains(c) { return get().includes(c); },
        replace(a, b) { set(get().map((x) => (x === a ? b : x))); },
        get length() { return get().length; },
        item(i) { return get()[i] ?? null; },
        toString() { return el.className; },
      };
    }
    get dataset() {
      const el = this;
      return new Proxy({}, {
        get: (_, k) => (typeof k === "string" ? el.getAttribute("data-" + kebab(k)) ?? undefined : undefined),
        set: (_, k, v) => { el.setAttribute("data-" + kebab(String(k)), v); return true; },
      });
    }
    get style() {
      const el = this;
      const read = () => {
        const m = {};
        for (const d of (el.getAttribute("style") || "").split(";")) {
          const i = d.indexOf(":");
          if (i > 0) m[d.slice(0, i).trim()] = d.slice(i + 1).trim();
        }
        return m;
      };
      const write = (m) => el.setAttribute("style", Object.entries(m).filter(([, v]) => v !== "").map(([k, v]) => k + ": " + v).join("; "));
      return new Proxy({}, {
        get(_, k) {
          if (k === "setProperty") return (p, v) => { const m = read(); m[p] = String(v); write(m); };
          if (k === "removeProperty") return (p) => { const m = read(); delete m[p]; write(m); };
          if (k === "getPropertyValue") return (p) => read()[p] || "";
          if (k === "cssText") return el.getAttribute("style") || "";
          return typeof k === "string" ? read()[kebab(k)] || "" : undefined;
        },
        set(_, k, v) {
          if (k === "cssText") { el.setAttribute("style", String(v)); return true; }
          const m = read();
          m[kebab(String(k))] = v == null ? "" : String(v);
          write(m);
          return true;
        },
      });
    }
    get children() { return this.childNodes.filter((n) => n.nodeType === 1); }
    get childElementCount() { return this.children.length; }
    get firstElementChild() { return this.children[0] || null; }
    get lastElementChild() { const c = this.children; return c[c.length - 1] || null; }
    get nextElementSibling() { let n = this.nextSibling; while (n && n.nodeType !== 1) n = n.nextSibling; return n; }
    get previousElementSibling() { let n = this.previousSibling; while (n && n.nodeType !== 1) n = n.previousSibling; return n; }
    get innerHTML() { return __getHTML(this.__id, false); }
    set innerHTML(v) { __setHTML(this.__id, String(v)); }
    get outerHTML() { return __getHTML(this.__id, true); }
    get innerText() { return this.textContent; }
    set innerText(v) { this.textContent = v; }
    insertAdjacentHTML(pos, html) {
      const tmp = document.createElement("div");
      tmp.innerHTML = html;
      this.__adjacent(pos, tmp.childNodes);
    }
    insertAdjacentElement(pos, el) { this.__adjacent(pos, [el]); return el; }
    insertAdjacentText(pos, t) { this.__adjacent(pos, [document.createTextNode(t)]); }
    __adjacent(pos, nodes) {
      const p = pos.toLowerCase();
      for (const n of nodes) {
        if (p === "beforebegin") this.parentNode && this.parentNode.insertBefore(n, this);
        else if (p === "afterbegin") this.insertBefore(n, this.firstChild);
        else if (p === "beforeend") this.appendChild(n);
        else if (p === "afterend") this.parentNode && this.parentNode.insertBefore(n, this.nextSibling);
      }
    }
    append(...a) { for (const n of nodesFrom(a)) this.appendChild(n); }
    prepend(...a) { const f = this.firstChild; for (const n of nodesFrom(a)) this.insertBefore(n, f); }
    before(...a) { for (const n of nodesFrom(a)) this.parentNode && this.parentNode.insertBefore(n, this); }
    after(...a) { const nx = this.nextSibling; for (const n of nodesFrom(a)) this.parentNode && this.parentNode.insertBefore(n, nx); }
    replaceWith(...a) { this.before(...a); this.remove(); }
    replaceChildren(...a) { __setText(this.__id, ""); this.append(...a); }
    remove() { __detach(this.__id); }
    querySelector(s) { return wrap(__query(this.__id, String(s), false)[0]); }
    querySelectorAll(s) { return __query(this.__id, String(s), true).map(wrap); }
    matches(s) { return __matches(this.__id, String(s)); }
    closest(s) { let n = this; while (n && n.nodeType === 1) { if (n.matches(s)) return n; n = n.parentNode; } return null; }
    getElementsByTagName(t) { return this.querySelectorAll(t === "*" ? "*" : t); }
    getElementsByClassName(c) { return this.querySelectorAll(c.trim().split(/\s+/).map((x) => "." + x).join("")); }
    getBoundingClientRect() { return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
    getClientRects() { return []; }
    get offsetWidth() { return 0; }
    get offsetHeight() { return 0; }
    get clientWidth() { return 0; }
    get clientHeight() { return 0; }
    scrollIntoView() {}
    focus() {}
    blur() {}
    click() { this.dispatchEvent(new Event("click", { bubbles: true, cancelable: true })); }
    attachShadow() { return this; }
    // Reflected attributes commonly used by scripts
    get href() { return this.getAttribute("href") || ""; }
    set href(v) { this.setAttribute("href", v); }
    get src() { return this.getAttribute("src") || ""; }
    set src(v) { this.setAttribute("src", v); }
    get value() { return this.getAttribute("value") || ""; }
    set value(v) { this.setAttribute("value", v); }
    get title() { return this.getAttribute("title") || ""; }
    set title(v) { this.setAttribute("title", v); }
    get hidden() { return this.hasAttribute("hidden"); }
    set hidden(v) { this.toggleAttribute("hidden", !!v); }
    get disabled() { return this.hasAttribute("disabled"); }
    set disabled(v) { this.toggleAttribute("disabled", !!v); }
    get checked() { return this.hasAttribute("checked"); }
    set checked(v) { this.toggleAttribute("checked", !!v); }
  }
  class HTMLElement extends Element {}

  class DocumentFragment extends Node {
    get nodeName() { return "#document-fragment"; }
    querySelector(s) { return Element.prototype.querySelector.call(this, s); }
    querySelectorAll(s) { return Element.prototype.querySelectorAll.call(this, s); }
    append(...a) { for (const n of nodesFrom(a)) this.appendChild(n); }
  }

  class Document extends Node {
    get nodeName() { return "#document"; }
    get documentElement() { return this.querySelector("html"); }
    get head() { return this.querySelector("head"); }
    get body() { return this.querySelector("body"); }
    get title() { const t = this.querySelector("title"); return t ? t.textContent.trim() : ""; }
    set title(v) {
      let t = this.querySelector("title");
      if (!t) { t = this.createElement("title"); (this.head || this.documentElement || this).appendChild(t); }
      t.textContent = v;
    }
    get readyState() { return __readyState; }
    get location() { return g.location; }
    get URL() { return __url; }
    get cookie() { return __cookie; }
    set cookie(v) { const kv = String(v).split(";")[0]; __cookie = __cookie ? __cookie + "; " + kv : kv; }
    get defaultView() { return g; }
    get activeElement() { return this.body; }
    get forms() { return this.querySelectorAll("form"); }
    get links() { return this.querySelectorAll("a[href]"); }
    get images() { return this.querySelectorAll("img"); }
    get scripts() { return this.querySelectorAll("script"); }
    get currentScript() { return wrap(__currentScript); }
    createElement(t) { return wrap(__create(String(t).toLowerCase())); }
    createElementNS(_ns, t) { return this.createElement(t); }
    createTextNode(s) { return wrap(__createText(String(s))); }
    createComment() { return wrap(__createText("")); }
    createDocumentFragment() { const f = new DocumentFragment(__create("#fragment")); cache.set(f.__id, f); return f; }
    createEvent() { return new Event(""); }
    getElementById(id) { return wrap(__byId(String(id))); }
    querySelector(s) { return wrap(__query(0, String(s), false)[0]); }
    querySelectorAll(s) { return __query(0, String(s), true).map(wrap); }
    getElementsByTagName(t) { return this.querySelectorAll(t); }
    getElementsByClassName(c) { return Element.prototype.getElementsByClassName.call(this, c); }
    getElementsByName(n) { return this.querySelectorAll('[name="' + n + '"]'); }
    write(...html) { __write(html.join("")); }
    writeln(...html) { __write(html.join("") + "\n"); }
    hasFocus() { return true; }
  }

  const document = new Document(0);
  cache.set(0, document);

  // ------------------------------------------------------------ timers
  let __timerId = 0;
  const timers = new Map();
  function addTimer(fn, ms, args, repeat) {
    const id = ++__timerId;
    const delay = Math.max(0, Number(ms) || 0);
    timers.set(id, { fn, args, due: __nowMs + delay, delay, repeat });
    return id;
  }
  g.setTimeout = (fn, ms, ...args) => addTimer(fn, ms, args, false);
  g.setInterval = (fn, ms, ...args) => addTimer(fn, Math.max(4, Number(ms) || 0), args, true);
  g.clearTimeout = g.clearInterval = (id) => { timers.delete(id); };
  g.requestAnimationFrame = (fn) => addTimer(() => fn(__nowMs), 16, [], false);
  g.cancelAnimationFrame = g.clearTimeout;
  g.queueMicrotask = (fn) => Promise.resolve().then(fn);
  g.requestIdleCallback = (fn) => addTimer(() => fn({ timeRemaining: () => 10, didTimeout: false }), 1, [], false);

  // Called from Rust: advance the clock and run timers that are due. Returns how many ran.
  g.__tick = function (now) {
    __nowMs = now;
    let ran = 0;
    for (let guard = 0; guard < 1000; guard++) {
      let next = null;
      for (const [id, t] of timers) if (t.due <= now && (!next || t.due < next[1].due)) next = [id, t];
      if (!next) break;
      const [id, t] = next;
      if (t.repeat) t.due = Math.max(t.due + t.delay, now - t.delay + 1);
      else timers.delete(id);
      ran++;
      try { typeof t.fn === "function" ? t.fn(...t.args) : (0, eval)(String(t.fn)); }
      catch (e) { console.error(String(e)); }
    }
    return ran;
  };
  g.__pendingTimers = () => timers.size;

  // Called from Rust when a node is clicked. Returns true if the default action should run.
  g.__dispatch = function (id, type) {
    const target = wrap(id);
    if (!target) return true;
    return target.dispatchEvent(new Event(type, { bubbles: true, cancelable: true }));
  };
  g.__loaded = function () {
    __readyState = "interactive";
    document.dispatchEvent(new Event("DOMContentLoaded", { bubbles: true }));
    __readyState = "complete";
    EventTarget.prototype.__fire.call(g, new Event("load"));
    document.dispatchEvent(new Event("readystatechange"));
  };

  // ------------------------------------------------------------ window
  Object.setPrototypeOf(g, EventTarget.prototype);
  const u = (() => {
    const m = /^([a-z]+:)\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(__url) || [];
    return { protocol: m[1] || "about:", host: m[2] || "", pathname: m[3] || "/", search: m[4] || "", hash: m[5] || "" };
  })();
  const location = {
    href: __url, protocol: u.protocol, host: u.host, hostname: u.host.split(":")[0],
    port: u.host.split(":")[1] || "", pathname: u.pathname, search: u.search, hash: u.hash,
    origin: u.protocol + "//" + u.host,
    assign(url) { __navigate(String(url)); }, replace(url) { __navigate(String(url)); },
    reload() { __navigate(__url); }, toString() { return __url; },
  };
  const storage = () => {
    const m = new Map();
    return {
      getItem: (k) => (m.has(String(k)) ? m.get(String(k)) : null),
      setItem: (k, v) => m.set(String(k), String(v)),
      removeItem: (k) => m.delete(String(k)),
      clear: () => m.clear(),
      key: (i) => [...m.keys()][i] ?? null,
      get length() { return m.size; },
    };
  };
  const consoleFn = (level) => (...a) => __log(level, a.map((x) => {
    if (typeof x === "string") return x;
    try { return JSON.stringify(x) ?? String(x); } catch { return String(x); }
  }).join(" "));

  Object.assign(g, {
    window: g, self: g, top: g, parent: g, globalThis: g, frames: g,
    document, location, Node, Element, HTMLElement, Text, Comment, Document, DocumentFragment,
    CharacterData, Event, CustomEvent, EventTarget,
    HTMLDivElement: HTMLElement, HTMLAnchorElement: HTMLElement, HTMLImageElement: HTMLElement,
    HTMLInputElement: HTMLElement, HTMLButtonElement: HTMLElement, HTMLScriptElement: HTMLElement,
    navigator: { userAgent: __userAgent, language: "en-US", languages: ["en-US", "en"], platform: "Linux x86_64",
                 onLine: true, cookieEnabled: true, hardwareConcurrency: 2, maxTouchPoints: 0, vendor: "" },
    screen: { width: 1366, height: 768, availWidth: 1366, availHeight: 768, colorDepth: 24 },
    innerWidth: 1024, innerHeight: 768, outerWidth: 1024, outerHeight: 800, devicePixelRatio: 1,
    scrollX: 0, scrollY: 0, pageXOffset: 0, pageYOffset: 0,
    scrollTo() {}, scrollBy() {}, scroll() {}, focus() {}, blur() {}, print() {}, open() { return null; }, close() {},
    alert: (m) => __log("alert", String(m)), confirm: () => false, prompt: () => null,
    getComputedStyle: (el) => el.style,
    matchMedia: (q) => ({ matches: false, media: q, addListener() {}, removeListener() {},
                          addEventListener() {}, removeEventListener() {} }),
    localStorage: storage(), sessionStorage: storage(),
    history: { length: 1, state: null, pushState() {}, replaceState() {}, back() {}, forward() {}, go() {} },
    performance: { now: () => __nowMs, mark() {}, measure() {}, getEntriesByType: () => [], timing: {} },
    console: { log: consoleFn("log"), info: consoleFn("info"), debug: consoleFn("debug"),
               warn: consoleFn("warn"), error: consoleFn("error"), trace: consoleFn("trace"),
               dir: consoleFn("log"), table: consoleFn("log"), group() {}, groupEnd() {}, time() {}, timeEnd() {} },
    MutationObserver: class { observe() {} disconnect() {} takeRecords() { return []; } },
    IntersectionObserver: class { observe() {} unobserve() {} disconnect() {} },
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    CSS: { supports: () => false, escape: (s) => String(s).replace(/([^\w-])/g, "\\$1") },
  });
})();
