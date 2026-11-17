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
    getBoundingClientRect() {
      const [x, y, w, h] = __rect(this.__id).split(",").map(Number);
      return { x, y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h, toJSON() { return this; } };
    }
    getClientRects() { const r = this.getBoundingClientRect(); return r.width || r.height ? [r] : []; }
    get offsetWidth() { return Math.round(this.getBoundingClientRect().width); }
    get offsetHeight() { return Math.round(this.getBoundingClientRect().height); }
    get offsetTop() { return Math.round(this.getBoundingClientRect().top + g.scrollY); }
    get offsetLeft() { return Math.round(this.getBoundingClientRect().left); }
    get offsetParent() { return document.body; }
    get clientWidth() {
      if (this === document.documentElement) return g.innerWidth;
      return Math.round(this.getBoundingClientRect().width);
    }
    get clientHeight() {
      if (this === document.documentElement) return g.innerHeight;
      const e = __elScroll(this.__id);
      if (e) return Math.round(Number(e.split(",")[2]));
      return Math.round(this.getBoundingClientRect().height);
    }
    get clientTop() { return 0; }
    get clientLeft() { return 0; }
    get scrollWidth() { return this === document.documentElement || this === document.body ? g.innerWidth : this.clientWidth; }
    get scrollHeight() {
      if (this === document.documentElement || this === document.body) return Math.round(Number(__viewport().split(",")[3]));
      const e = __elScroll(this.__id);
      if (e) { const [, max, h] = e.split(",").map(Number); return Math.round(max + h); }
      return this.clientHeight;
    }
    get scrollTop() {
      if (this === document.documentElement || this === document.body) return g.scrollY;
      const e = __elScroll(this.__id);
      return e ? Number(e.split(",")[0]) : 0;
    }
    set scrollTop(v) {
      if (this === document.documentElement || this === document.body) __scrollTo(Number(v));
      else __setElScroll(this.__id, Number(v));
    }
    scrollBy(a, b) { const dy = typeof a === "object" && a ? Number(a.top || 0) : Number(b || 0); this.scrollTop = this.scrollTop + dy; }
    scrollTo(a, b) { this.scrollTop = typeof a === "object" && a ? Number(a.top || 0) : Number(b || 0); }
    get scrollLeft() { return 0; }
    set scrollLeft(v) {}
    scrollIntoView(opts) {
      const r = this.getBoundingClientRect();
      let y = g.scrollY + r.top;
      const block = opts && typeof opts === "object" ? opts.block : opts === false ? "end" : "start";
      if (block === "center") y -= (g.innerHeight - r.height) / 2;
      else if (block === "end") y -= g.innerHeight - r.height;
      __scrollTo(y);
    }
    focus() { __request("focus", this.__id); }
    blur() { __request("blur", this.__id); }
    click() { this.dispatchEvent(new Event("click", { bubbles: true, cancelable: true })); }
    attachShadow() { return this; }
    // Reflected attributes commonly used by scripts
    get href() { return this.getAttribute("href") || ""; }
    set href(v) { this.setAttribute("href", v); }
    get src() { return this.getAttribute("src") || ""; }
    set src(v) { this.setAttribute("src", v); }
    get value() {
      const t = this.tagName;
      if (t === "TEXTAREA") return this.textContent.replace(/^\n/, "");
      if (t === "SELECT") { const o = this.options[this.selectedIndex]; return o ? o.value : ""; }
      if (t === "OPTION") return this.hasAttribute("value") ? this.getAttribute("value") : this.textContent.trim();
      return this.getAttribute("value") || "";
    }
    set value(v) {
      const t = this.tagName;
      if (t === "TEXTAREA") { this.textContent = String(v); return; }
      if (t === "SELECT") { this.options.forEach((o) => { o.selected = o.value === String(v); }); return; }
      this.setAttribute("value", v);
    }
    get defaultValue() { return this.tagName === "TEXTAREA" ? this.textContent : this.getAttribute("value") || ""; }
    get name() { return this.getAttribute("name") || ""; }
    set name(v) { this.setAttribute("name", v); }
    get type() {
      const t = this.tagName;
      if (t === "SELECT") return this.hasAttribute("multiple") ? "select-multiple" : "select-one";
      if (t === "TEXTAREA") return "textarea";
      if (t === "BUTTON") return (this.getAttribute("type") || "submit").toLowerCase();
      return (this.getAttribute("type") || "text").toLowerCase();
    }
    set type(v) { this.setAttribute("type", v); }
    get placeholder() { return this.getAttribute("placeholder") || ""; }
    set placeholder(v) { this.setAttribute("placeholder", v); }
    get selected() { return this.hasAttribute("selected"); }
    set selected(v) { this.toggleAttribute("selected", !!v); }
    get options() { return Array.from(this.querySelectorAll("option")); }
    get selectedIndex() {
      const o = this.options;
      for (let i = o.length - 1; i >= 0; i--) if (o[i].selected) return i;
      return o.length ? 0 : -1;
    }
    set selectedIndex(i) { this.options.forEach((o, j) => { o.selected = j === i; }); }
    get form() {
      const f = this.getAttribute("form");
      if (f) return document.getElementById(f);
      return this.closest ? this.closest("form") : null;
    }
    get elements() { return this.tagName === "FORM" ? Array.from(document.querySelectorAll("input,select,textarea,button")).filter((e) => e.form === this) : []; }
    get action() { return this.getAttribute("action") || location.href; }
    set action(v) { this.setAttribute("action", v); }
    get method() { return (this.getAttribute("method") || "get").toLowerCase(); }
    set method(v) { this.setAttribute("method", v); }
    submit() { __request("submit", this.__id, -1); }
    requestSubmit(by) { __request("requestSubmit", this.__id, by ? by.__id : -1); }
    reset() { __request("reset", this.__id); }
    select() {}
    setSelectionRange() {}
    checkValidity() { return true; }
    reportValidity() { return true; }
    setCustomValidity() {}
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
    // Real cookies for web pages; a simple per-page store for local files.
    get cookie() { return /^https?:/.test(__url) ? __getCookie(__url) : __cookie; }
    set cookie(v) {
      if (/^https?:/.test(__url)) { __setCookie(__url, String(v)); return; }
      const kv = String(v).split(";")[0], name = kv.split("=")[0];
      const rest = __cookie ? __cookie.split("; ").filter((p) => p.split("=")[0] !== name) : [];
      if (!/max-age=(0|-)|expires=thu, 01 jan 1970/i.test(String(v))) rest.push(kv);
      __cookie = rest.join("; ");
    }
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

  // A separate, blank document (document.implementation.createHTMLDocument, DOMParser).
  // Its nodes live in the same tree storage, detached from the page.
  class DetachedDocument extends Document {
    constructor(title) {
      const html = document.createElement("html");
      super(html.__id);
      this._root = html;
      const head = document.createElement("head"), body = document.createElement("body");
      html.appendChild(head);
      html.appendChild(body);
      if (title !== undefined) { const t = document.createElement("title"); t.textContent = String(title); head.appendChild(t); }
    }
    get documentElement() { return this._root; }
    get head() { return this._root.querySelector("head"); }
    get body() { return this._root.querySelector("body"); }
    get readyState() { return "complete"; }
    get defaultView() { return null; }
    getElementById(id) { return this._root.querySelector("#" + CSS.escape(String(id))); }
    querySelector(sel) { return this._root.querySelector(sel); }
    querySelectorAll(sel) { return this._root.querySelectorAll(sel); }
    write(...html) { this.body.insertAdjacentHTML ? this.body.insertAdjacentHTML("beforeend", html.join("")) : (this.body.innerHTML += html.join("")); }
  }
  document.implementation = {
    createHTMLDocument: (title) => new DetachedDocument(title),
    createDocument: () => new DetachedDocument(),
    hasFeature: () => true,
  };
  class DOMParser {
    parseFromString(text, type) {
      const d = new DetachedDocument();
      const src = String(text);
      const head = /<head[^>]*>([\s\S]*?)<\/head>/i.exec(src);
      const body = /<body[^>]*>([\s\S]*)<\/body>/i.exec(src);
      if (head) d.head.innerHTML = head[1];
      d.body.innerHTML = body ? body[1] : head ? src.replace(head[0], "").replace(/<\/?html[^>]*>/gi, "") : src;
      return d;
    }
  }

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
  g.queueMicrotask = (fn) => Promise.resolve().then(fn);
  g.requestIdleCallback = (fn) => addTimer(() => fn({ timeRemaining: () => 10, didTimeout: false }), 1, [], false);
  g.cancelIdleCallback = g.clearTimeout;

  // The event loop. Rust runs one task per call (a queued message, else the earliest due
  // timer) and runs promise callbacks (microtasks) after each, as browsers do.
  const tasks = [];
  g.__queueTask = (fn) => { tasks.push(fn); };
  g.__runTask = function (now) {
    __nowMs = now;
    if (tasks.length) {
      const fn = tasks.shift();
      try { fn(); } catch (e) { console.error(String(e)); }
      return 1;
    }
    let next = null;
    for (const [id, t] of timers) if (t.due <= now && (!next || t.due < next[1].due)) next = [id, t];
    if (!next) return 0;
    const [id, t] = next;
    if (t.repeat) t.due = Math.max(t.due + t.delay, now - t.delay + 1);
    else timers.delete(id);
    try { typeof t.fn === "function" ? t.fn(...t.args) : (0, eval)(String(t.fn)); }
    catch (e) { console.error(String(e)); }
    return 1;
  };
  // Animation frames: callbacks run once per drawn frame, all with the same timestamp.
  const frames = new Map();
  let frameId = 0;
  g.requestAnimationFrame = (fn) => { const id = ++frameId; frames.set(id, fn); return id; };
  g.cancelAnimationFrame = (id) => { frames.delete(id); };
  g.__hasFrames = () => frames.size > 0;
  // Milliseconds until the next timer is due (-1 if none), for headless rendering.
  g.__nextTimer = () => { let m = -1; for (const t of timers.values()) if (!t.repeat && (m < 0 || t.due < m)) m = t.due; return m < 0 ? -1 : Math.max(0, m - __nowMs); };
  g.__runFrame = function (now) {
    __nowMs = now;
    const due = [...frames.values()];
    frames.clear();
    for (const fn of due) { try { fn(now); } catch (e) { console.error(String(e)); } }
    return due.length;
  };
  // Kept for older callers: run everything that's due in one go.
  g.__tick = function (now) { let n = 0; while (n < 1000 && g.__runTask(now)) n++; return n; };
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
  // location reflects the current URL, which pushState and in-page links change.
  const parts = () => {
    const m = /^([a-z][a-z0-9+.-]*:)(?:\/\/([^/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(__url) || [];
    return { protocol: (m[1] || "about:").toLowerCase(), host: m[2] || "", pathname: m[3] || "/", search: m[4] && m[4] !== "?" ? m[4] : "", hash: m[5] && m[5] !== "#" ? m[5] : "" };
  };
  const sameDoc = (a, b) => a.split("#")[0] === b.split("#")[0];
  const go = (url, replace) => {
    const abs = __resolve(__url, String(url));
    if (sameDoc(abs, __url) && abs.includes("#")) __history(replace ? "hash-replace" : "hash", abs, "");
    else __navigate(abs);
  };
  const withPart = (k, v) => { const p = parts(); p[k] = v; return p.protocol + (p.host || p.protocol === "file:" ? "//" : "") + p.host + p.pathname + p.search + p.hash; };
  const location = {
    get href() { return __url; }, set href(v) { go(v); },
    get protocol() { return parts().protocol; },
    get host() { return parts().host; },
    get hostname() { return parts().host.replace(/:\d+$/, ""); },
    get port() { const m = /:(\d+)$/.exec(parts().host); return m ? m[1] : ""; },
    get pathname() { return parts().pathname; }, set pathname(v) { go(withPart("pathname", "/" + String(v).replace(/^\//, ""))); },
    get search() { return parts().search; }, set search(v) { const s = String(v).replace(/^\?/, ""); go(withPart("search", s ? "?" + s : "")); },
    get hash() { return parts().hash; }, set hash(v) { const h = String(v).replace(/^#/, ""); go(__url.split("#")[0] + "#" + h); },
    get origin() { const p = parts(); return /^https?:$/.test(p.protocol) ? p.protocol + "//" + p.host : "null"; },
    assign(url) { go(url); }, replace(url) { go(url, true); },
    reload() { __navigate(__url); }, toString() { return __url; },
  };
  // History: entries pushed by the page share this document; the browser keeps the list.
  let histState = null, histLength = 1;
  const clone = (v) => (v === undefined ? null : JSON.parse(JSON.stringify(v)));
  const history = {
    get length() { return histLength; },
    get state() { return histState; },
    scrollRestoration: "auto",
    pushState(state, _title, url) { change("push", state, url); histLength++; },
    replaceState(state, _title, url) { change("replace", state, url); },
    back() { __history("go", "-1", ""); }, forward() { __history("go", "1", ""); },
    go(n) { __history("go", String(Number(n) || 0), ""); },
  };
  function change(kind, state, url) {
    const abs = url == null || url === "" ? __url : __resolve(__url, String(url));
    const origin = (x) => { const m = /^([a-z]+:)\/\/([^/?#]*)/i.exec(x); return m ? m[1] + m[2] : x.split(/[?#]/)[0]; };
    if (origin(abs) !== origin(__url)) throw new DOMException("pushState: URL must be on the same origin", "SecurityError");
    histState = clone(state);
    __url = abs;
    __history(kind, abs, JSON.stringify(histState));
  }
  // Called by the browser when it moves within this document's history (Back, Forward, #links).
  g.__historyMoved = function (url, stateJson, popstate) {
    const old = __url;
    __url = url;
    if (popstate) {
      histState = stateJson ? JSON.parse(stateJson) : null;
      const ev = new Event("popstate"); ev.state = histState;
      EventTarget.prototype.__fire.call(g, ev);
    }
    if (old.split("#")[1] !== url.split("#")[1] && sameDoc(old, url)) {
      const ev = new Event("hashchange"); ev.oldURL = old; ev.newURL = url;
      EventTarget.prototype.__fire.call(g, ev);
    }
  };
  // Web Storage. localStorage is saved per site (origin) in the profile folder and kept
  // between runs; sessionStorage lasts as long as the page.
  const storage = (persist) => {
    const origin = location.origin === "null" ? "file" : location.origin;
    let m = new Map();
    if (persist) {
      try { m = new Map(Object.entries(JSON.parse(__storageLoad(origin)))); } catch (e) { m = new Map(); }
    }
    const save = () => {
      if (persist && !__storageSave(origin, JSON.stringify(Object.fromEntries(m)))) {
        throw new DOMException("The quota has been exceeded.", "QuotaExceededError");
      }
    };
    const api = {
      getItem: (k) => (m.has(String(k)) ? m.get(String(k)) : null),
      setItem: (k, v) => { const had = m.get(String(k)); m.set(String(k), String(v)); try { save(); } catch (e) { if (had === undefined) m.delete(String(k)); else m.set(String(k), had); throw e; } },
      removeItem: (k) => { if (m.delete(String(k))) save(); },
      clear: () => { if (m.size) { m.clear(); save(); } },
      key: (i) => [...m.keys()][i] ?? null,
      get length() { return m.size; },
    };
    // Also allow localStorage.foo = "bar" and localStorage.foo, as pages do.
    return new Proxy(api, {
      get: (t, k) => (k in t || typeof k === "symbol" ? t[k] : t.getItem(k)),
      set: (t, k, v) => { if (k in t) return false; t.setItem(k, v); return true; },
      deleteProperty: (t, k) => { t.removeItem(k); return true; },
      has: (t, k) => k in t || m.has(String(k)),
      ownKeys: () => [...m.keys()],
      getOwnPropertyDescriptor: (t, k) => (m.has(String(k)) ? { value: m.get(String(k)), enumerable: true, configurable: true, writable: true } : undefined),
    });
  };
  const consoleFn = (level) => (...a) => __log(level, a.map((x) => {
    if (typeof x === "string") return x;
    try { return JSON.stringify(x) ?? String(x); } catch { return String(x); }
  }).join(" "));


  // ------------------------------------------------------------ URL, fetch, XMLHttpRequest
  if (typeof g.URLSearchParams === "undefined") {
    const dec = (s) => { try { return decodeURIComponent(s.replace(/\+/g, " ")); } catch (e) { return s; } };
    const enc = (s) => encodeURIComponent(s).replace(/%20/g, "+").replace(/[!'()~]/g, (c) => "%" + c.charCodeAt(0).toString(16).toUpperCase());
    g.URLSearchParams = class URLSearchParams {
      constructor(init) {
        this._p = [];
        if (init == null) return;
        if (typeof init === "string") {
          for (const part of init.replace(/^\?/, "").split("&")) {
            if (!part) continue;
            const i = part.indexOf("=");
            this._p.push(i < 0 ? [dec(part), ""] : [dec(part.slice(0, i)), dec(part.slice(i + 1))]);
          }
        } else if (init instanceof URLSearchParams || Array.isArray(init) || (init && typeof init[Symbol.iterator] === "function")) {
          for (const [k, v] of init) this._p.push([String(k), String(v)]);
        } else {
          for (const k of Object.keys(init)) this._p.push([k, String(init[k])]);
        }
      }
      get size() { return this._p.length; }
      append(k, v) { this._p.push([String(k), String(v)]); this._u && this._u(); }
      delete(k) { this._p = this._p.filter(([a]) => a !== k); this._u && this._u(); }
      get(k) { const e = this._p.find(([a]) => a === k); return e ? e[1] : null; }
      getAll(k) { return this._p.filter(([a]) => a === k).map(([, v]) => v); }
      has(k) { return this._p.some(([a]) => a === k); }
      set(k, v) {
        const i = this._p.findIndex(([a]) => a === k);
        if (i < 0) this._p.push([String(k), String(v)]);
        else { this._p[i][1] = String(v); this._p = this._p.filter(([a], j) => a !== k || j === i); }
        this._u && this._u();
      }
      sort() { this._p.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)); this._u && this._u(); }
      forEach(fn, t) { for (const [k, v] of this._p) fn.call(t, v, k, this); }
      keys() { return this._p.map(([k]) => k)[Symbol.iterator](); }
      values() { return this._p.map(([, v]) => v)[Symbol.iterator](); }
      entries() { return this._p.map((e) => e.slice())[Symbol.iterator](); }
      [Symbol.iterator]() { return this.entries(); }
      toString() { return this._p.map(([k, v]) => enc(k) + "=" + enc(v)).join("&"); }
    };
  }
  if (typeof g.URL === "undefined") {
    const RE = /^([a-zA-Z][a-zA-Z0-9+.-]*:)(?:\/\/(?:([^:@\/]*)(?::([^@\/]*))?@)?([^:\/?#]*)(?::(\d+))?)?([^?#]*)(\?[^#]*)?(#.*)?$/;
    g.URL = class URL {
      constructor(url, base) {
        let abs = String(url);
        if (base !== undefined) abs = __resolve(String(base instanceof URL ? base.href : base), abs);
        const m = RE.exec(abs);
        if (!m) throw new TypeError("Invalid URL: " + url);
        this.protocol = m[1].toLowerCase(); this.username = m[2] || ""; this.password = m[3] || "";
        this.hostname = (m[4] || "").toLowerCase(); this.port = m[5] || "";
        this.pathname = m[6] || (this.hostname ? "/" : ""); this.hash = m[8] && m[8] !== "#" ? m[8] : "";
        const self = this;
        this.searchParams = new URLSearchParams(m[7] || "");
        this.searchParams._u = function () { self._search = this.toString(); };
        this._search = m[7] && m[7] !== "?" ? m[7].slice(1) : "";
        if ((this.protocol === "http:" && this.port === "80") || (this.protocol === "https:" && this.port === "443")) this.port = "";
      }
      get search() { return this._search ? "?" + this._search : ""; }
      set search(v) { this._search = String(v).replace(/^\?/, ""); const self = this; this.searchParams = new URLSearchParams(this._search); this.searchParams._u = function () { self._search = this.toString(); }; }
      get host() { return this.hostname + (this.port ? ":" + this.port : ""); }
      get origin() { return /^https?:$/.test(this.protocol) ? this.protocol + "//" + this.host : "null"; }
      get href() {
        const auth = this.username ? this.username + (this.password ? ":" + this.password : "") + "@" : "";
        const slashes = this.hostname || this.protocol === "file:" ? "//" : "";
        return this.protocol + slashes + auth + this.host + this.pathname + this.search + this.hash;
      }
      set href(v) { Object.assign(this, new URL(v)); }
      toString() { return this.href; }
      toJSON() { return this.href; }
      static canParse(u, b) { try { new URL(u, b); return true; } catch (e) { return false; } }
      static createObjectURL() { return "blob:null"; }
      static revokeObjectURL() {}
    };
  }

  class Headers {
    constructor(init) {
      this._h = [];
      if (init instanceof Headers) init = init._h;
      if (Array.isArray(init)) for (const [k, v] of init) this.append(k, v);
      else if (init) for (const k of Object.keys(init)) this.append(k, init[k]);
    }
    append(k, v) { this._h.push([String(k).toLowerCase(), String(v)]); }
    set(k, v) { k = String(k).toLowerCase(); this.delete(k); this._h.push([k, String(v)]); }
    get(k) { k = String(k).toLowerCase(); const v = this._h.filter(([a]) => a === k).map(([, b]) => b); return v.length ? v.join(", ") : null; }
    has(k) { k = String(k).toLowerCase(); return this._h.some(([a]) => a === k); }
    delete(k) { k = String(k).toLowerCase(); this._h = this._h.filter(([a]) => a !== k); }
    forEach(fn, t) { for (const [k, v] of this._h) fn.call(t, v, k, this); }
    keys() { return this._h.map(([k]) => k)[Symbol.iterator](); }
    values() { return this._h.map(([, v]) => v)[Symbol.iterator](); }
    entries() { return this._h.map((e) => e.slice())[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
  }

  class Response {
    constructor(body, init) {
      init = init || {};
      this._body = body == null ? "" : String(body);
      this.status = init.status === undefined ? 200 : init.status;
      this.statusText = init.statusText || "";
      this.headers = init.headers instanceof Headers ? init.headers : new Headers(init.headers);
      this.url = init.url || "";
      this.type = init.type || "basic";
      this.redirected = !!init.redirected;
      this.bodyUsed = false;
    }
    get ok() { return this.status >= 200 && this.status < 300; }
    _take() {
      if (this.bodyUsed) return Promise.reject(new TypeError("Body has already been consumed."));
      this.bodyUsed = true;
      return Promise.resolve(this._body);
    }
    text() { return this._take(); }
    json() { return this._take().then((t) => JSON.parse(t)); }
    arrayBuffer() { return this._take().then((t) => { const b = new Uint8Array(t.length); for (let i = 0; i < t.length; i++) b[i] = t.charCodeAt(i) & 255; return b.buffer; }); }
    blob() { return this._take().then((t) => ({ size: t.length, type: this.headers.get("content-type") || "", text: () => Promise.resolve(t) })); }
    formData() { return this._take().then((t) => { const f = new FormData(); for (const [k, v] of new URLSearchParams(t)) f.append(k, v); return f; }); }
    clone() { const r = new Response(this._body, this); r.url = this.url; return r; }
    static json(data, init) { const h = new Headers((init && init.headers) || {}); h.set("content-type", "application/json"); return new Response(JSON.stringify(data), Object.assign({}, init, { headers: h })); }
    static error() { return new Response("", { status: 0, type: "error" }); }
    static redirect(url, status) { return new Response("", { status: status || 302, headers: { location: url } }); }
  }

  class Request {
    constructor(input, init) {
      init = init || {};
      if (input instanceof Request) { init = Object.assign({ method: input.method, headers: input.headers, body: input._body, credentials: input.credentials, signal: input.signal }, init); input = input.url; }
      this.url = __resolve(location.href, String(input));
      this.method = String(init.method || "GET").toUpperCase();
      this.headers = new Headers(init.headers);
      this._body = init.body;
      this.credentials = init.credentials || "same-origin";
      this.signal = init.signal || null;
      this.mode = init.mode || "cors";
    }
    clone() { return new Request(this); }
  }

  class FormData {
    constructor(form) {
      this._p = [];
      if (form && form.__id !== undefined) for (const [k, v] of JSON.parse(__formData(form.__id))) this._p.push([k, v]);
    }
    append(k, v) { this._p.push([String(k), String(v)]); }
    set(k, v) { this.delete(k); this.append(k, v); }
    get(k) { const e = this._p.find(([a]) => a === k); return e ? e[1] : null; }
    getAll(k) { return this._p.filter(([a]) => a === k).map(([, v]) => v); }
    has(k) { return this._p.some(([a]) => a === k); }
    delete(k) { this._p = this._p.filter(([a]) => a !== k); }
    forEach(fn, t) { for (const [k, v] of this._p) fn.call(t, v, k, this); }
    keys() { return this._p.map(([k]) => k)[Symbol.iterator](); }
    values() { return this._p.map(([, v]) => v)[Symbol.iterator](); }
    entries() { return this._p.map((e) => e.slice())[Symbol.iterator](); }
    [Symbol.iterator]() { return this.entries(); }
  }

  class AbortSignal extends EventTarget {
    constructor() { super(); this.aborted = false; this.reason = undefined; this.onabort = null; }
    throwIfAborted() { if (this.aborted) throw this.reason; }
    static abort(reason) { const c = new AbortController(); c.abort(reason); return c.signal; }
    static timeout(ms) { const c = new AbortController(); setTimeout(() => c.abort(new DOMException("The operation timed out.", "TimeoutError")), ms); return c.signal; }
  }
  class AbortController {
    constructor() { this.signal = new AbortSignal(); }
    abort(reason) {
      const s = this.signal;
      if (s.aborted) return;
      s.aborted = true;
      s.reason = reason === undefined ? new DOMException("The operation was aborted.", "AbortError") : reason;
      s.dispatchEvent(new Event("abort"));
    }
  }
  class DOMException extends Error {
    constructor(message, name) { super(message); this.name = name || "Error"; }
  }

  // Requests in flight: id -> callback(result). Results arrive through __netDone from Rust.
  const pending = new Map();
  let nextNet = 1;
  g.__netDone = function (id, result) {
    const cb = pending.get(id);
    pending.delete(id);
    if (cb) cb(result);
  };
  const flatHeaders = (h) => { let out = ""; h.forEach((v, k) => { out += k + "\n" + String(v).replace(/\n/g, " ") + "\n"; }); return out; };
  const bodyString = (b, headers) => {
    if (b == null) return null;
    if (b instanceof URLSearchParams) { if (!headers.has("content-type")) headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8"); return b.toString(); }
    if (b instanceof FormData) { if (!headers.has("content-type")) headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8"); return new URLSearchParams(b._p).toString(); }
    if (typeof b === "string") { if (!headers.has("content-type")) headers.set("content-type", "text/plain;charset=UTF-8"); return b; }
    if (b instanceof ArrayBuffer || ArrayBuffer.isView(b)) { const u = new Uint8Array(b.buffer || b); let s = ""; for (const c of u) s += String.fromCharCode(c); return s; }
    return String(b);
  };

  g.fetch = function (input, init) {
    let req;
    try { req = new Request(input, init); } catch (e) { return Promise.reject(e); }
    return new Promise((resolve, reject) => {
      if (req.signal && req.signal.aborted) return reject(req.signal.reason);
      const id = nextNet++;
      const body = bodyString(req._body, req.headers);
      pending.set(id, (r) => {
        if (r.error) { console.warn("fetch failed: " + r.error); return reject(new TypeError("Failed to fetch")); }
        const res = new Response(r.body, { status: r.status, statusText: r.statusText, headers: r.headers, url: r.url });
        res.redirected = r.url !== req.url;
        resolve(res);
      });
      if (req.signal) req.signal.addEventListener("abort", () => { if (pending.delete(id)) reject(req.signal.reason); });
      __netStart(id, location.href, req.method, req.url, flatHeaders(req.headers), body, req.credentials, false);
    });
  };

  class XMLHttpRequest extends EventTarget {
    constructor() {
      super();
      this.readyState = 0; this.status = 0; this.statusText = ""; this.responseText = ""; this.responseURL = "";
      this.responseType = ""; this.timeout = 0; this.withCredentials = false; this._headers = new Headers(); this._resHeaders = new Headers();
      this.upload = new EventTarget();
      for (const n of ["onreadystatechange", "onload", "onerror", "onabort", "onloadend", "onloadstart", "onprogress", "ontimeout"]) this[n] = null;
    }
    static get UNSENT() { return 0; } static get OPENED() { return 1; } static get HEADERS_RECEIVED() { return 2; } static get LOADING() { return 3; } static get DONE() { return 4; }
    _fire(type) { this.dispatchEvent(new Event(type)); } // dispatchEvent also runs this["on" + type]
    _state(n) { this.readyState = n; this._fire("readystatechange"); }
    open(method, url, async) {
      this._method = String(method).toUpperCase();
      this._url = __resolve(location.href, String(url));
      this._async = async !== false;
      this._id = 0;
      this._state(1);
    }
    setRequestHeader(k, v) { this._headers.append(k, v); }
    getResponseHeader(k) { return this.readyState >= 2 ? this._resHeaders.get(k) : null; }
    getAllResponseHeaders() { let s = ""; this._resHeaders.forEach((v, k) => { s += k + ": " + v + "\r\n"; }); return s; }
    overrideMimeType() {}
    get response() {
      if (this.responseType === "json") { try { return JSON.parse(this.responseText); } catch (e) { return null; } }
      if (this.responseType === "document") return null;
      return this.responseText;
    }
    _finish(r) {
      if (r.error) {
        console.warn("XMLHttpRequest failed: " + r.error);
        this.status = 0; this._state(4); this._fire("error"); this._fire("loadend");
        return;
      }
      this.status = r.status; this.statusText = r.statusText; this.responseURL = r.url;
      this._resHeaders = new Headers(r.headers);
      this._state(2);
      this.responseText = r.body;
      this._state(3);
      this._state(4);
      this._fire("load");
      this._fire("loadend");
    }
    send(body) {
      if (this.readyState !== 1) throw new DOMException("The object's state must be OPENED.", "InvalidStateError");
      const b = bodyString(body, this._headers);
      const cred = this.withCredentials ? "include" : "same-origin";
      this._fire("loadstart");
      if (!this._async) {
        this._finish(JSON.parse(__netStart(0, location.href, this._method, this._url, flatHeaders(this._headers), b, cred, true)));
        return;
      }
      const id = nextNet++;
      this._id = id;
      pending.set(id, (r) => this._finish(r));
      if (this.timeout > 0) setTimeout(() => { if (pending.delete(id)) { this.readyState = 4; this._fire("timeout"); this._fire("loadend"); } }, this.timeout);
      __netStart(id, location.href, this._method, this._url, flatHeaders(this._headers), b, cred, false);
    }
    abort() {
      if (this._id && pending.delete(this._id)) { this.readyState = 4; this._fire("abort"); this._fire("loadend"); }
      this.readyState = 0;
    }
  }


  // ------------------------------------------------------------ messaging
  if (typeof g.structuredClone !== "function") {
    g.structuredClone = (v) => {
      if (v === undefined) return undefined;
      return JSON.parse(JSON.stringify(v, (k, x) => (x instanceof Map ? Object.fromEntries(x) : x instanceof Set ? [...x] : x)));
    };
  }
  class MessageEvent extends Event {
    constructor(type, init) { super(type, init); init = init || {}; this.data = init.data; this.origin = init.origin || ""; this.source = init.source || null; this.ports = init.ports || []; this.lastEventId = ""; }
  }
  class MessagePort extends EventTarget {
    constructor() { super(); this.onmessage = null; this._other = null; this._closed = false; }
    postMessage(data) {
      const to = this._other;
      if (!to || this._closed) return;
      const copy = g.structuredClone(data);
      __queueTask(() => { if (!to._closed) to.dispatchEvent(new MessageEvent("message", { data: copy })); });
    }
    start() {}
    close() { this._closed = true; }
  }
  class MessageChannel {
    constructor() { this.port1 = new MessagePort(); this.port2 = new MessagePort(); this.port1._other = this.port2; this.port2._other = this.port1; }
  }
  g.postMessage = function (data, targetOrigin) {
    const copy = g.structuredClone(data);
    const t = typeof targetOrigin === "object" && targetOrigin ? targetOrigin.targetOrigin : targetOrigin;
    if (t && t !== "*" && t !== "/" && t !== location.origin) return;
    __queueTask(() => {
      const ev = new MessageEvent("message", { data: copy, origin: location.origin, source: g });
      EventTarget.prototype.__fire.call(g, ev);
    });
  };
  Object.assign(g, { MessageEvent, MessagePort, MessageChannel });

  // The standard element interfaces (HTMLInputElement, SVGElement…). Each matches only its
  // own tags with instanceof, so checks like `x instanceof HTMLIFrameElement` are honest.
  const SVG_TAGS = "svg g path circle ellipse line polyline polygon rect text tspan textpath defs use symbol clippath mask pattern image marker lineargradient radialgradient stop filter fegaussianblur feoffset feblend fecolormatrix foreignobject title desc switch view".split(" ");
  const interfaces = {
    HTMLAnchorElement: "a", HTMLAreaElement: "area", HTMLAudioElement: "audio", HTMLBRElement: "br", HTMLBaseElement: "base",
    HTMLBodyElement: "body", HTMLButtonElement: "button", HTMLCanvasElement: "canvas", HTMLDListElement: "dl",
    HTMLDataListElement: "datalist", HTMLDetailsElement: "details", HTMLDialogElement: "dialog", HTMLDivElement: "div",
    HTMLEmbedElement: "embed", HTMLFieldSetElement: "fieldset", HTMLFormElement: "form", HTMLHRElement: "hr",
    HTMLHeadElement: "head", HTMLHeadingElement: "h1 h2 h3 h4 h5 h6", HTMLHtmlElement: "html", HTMLIFrameElement: "iframe",
    HTMLImageElement: "img", HTMLInputElement: "input", HTMLLIElement: "li", HTMLLabelElement: "label", HTMLLegendElement: "legend",
    HTMLLinkElement: "link", HTMLMapElement: "map", HTMLMetaElement: "meta", HTMLMeterElement: "meter", HTMLOListElement: "ol",
    HTMLObjectElement: "object", HTMLOptGroupElement: "optgroup", HTMLOptionElement: "option", HTMLOutputElement: "output",
    HTMLParagraphElement: "p", HTMLPictureElement: "picture", HTMLPreElement: "pre", HTMLProgressElement: "progress",
    HTMLQuoteElement: "q blockquote", HTMLScriptElement: "script", HTMLSelectElement: "select", HTMLSlotElement: "slot",
    HTMLSourceElement: "source", HTMLSpanElement: "span", HTMLStyleElement: "style", HTMLTableElement: "table",
    HTMLTableCaptionElement: "caption", HTMLTableCellElement: "td th", HTMLTableColElement: "col colgroup",
    HTMLTableRowElement: "tr", HTMLTableSectionElement: "thead tbody tfoot", HTMLTemplateElement: "template",
    HTMLTextAreaElement: "textarea", HTMLTimeElement: "time", HTMLTitleElement: "title", HTMLTrackElement: "track",
    HTMLUListElement: "ul", HTMLVideoElement: "video", HTMLMediaElement: "audio video",
  };
  const isEl = (x) => x != null && typeof x === "object" && x.nodeType === 1;
  for (const [name, tags] of Object.entries(interfaces)) {
    const set = new Set(tags.split(" "));
    const C = class extends HTMLElement { static [Symbol.hasInstance](x) { return isEl(x) && set.has(x.localName || x.tagName.toLowerCase()); } };
    Object.defineProperty(C, "name", { value: name });
    g[name] = C;
  }
  const svgSet = new Set(SVG_TAGS);
  const isSvg = (x) => isEl(x) && svgSet.has((x.localName || x.tagName).toLowerCase());
  g.SVGElement = class SVGElement extends Element { static [Symbol.hasInstance](x) { return isSvg(x); } };
  g.SVGGraphicsElement = class SVGGraphicsElement extends g.SVGElement { static [Symbol.hasInstance](x) { return isSvg(x); } };
  g.SVGSVGElement = class SVGSVGElement extends g.SVGElement { static [Symbol.hasInstance](x) { return isEl(x) && x.tagName.toLowerCase() === "svg"; } };
  g.HTMLUnknownElement = class HTMLUnknownElement extends HTMLElement { static [Symbol.hasInstance]() { return false; } };
  g.ShadowRoot = class ShadowRoot extends DocumentFragment {};
  g.HTMLDocument = Document;
  g.NodeList = Array;
  g.HTMLCollection = Array;
  g.NodeFilter = { SHOW_ALL: -1, SHOW_ELEMENT: 1, SHOW_TEXT: 4, SHOW_COMMENT: 128, FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3 };

  Object.assign(g, { Headers, Response, Request, FormData, AbortController, AbortSignal, DOMException, XMLHttpRequest });

  Object.assign(g, {
    window: g, self: g, top: g, parent: g, globalThis: g, frames: g,
    document, location, Node, Element, HTMLElement, Text, Comment, Document, DocumentFragment,
    CharacterData, Event, CustomEvent, EventTarget,
    DOMParser,
    navigator: { userAgent: __userAgent, language: "en-US", languages: ["en-US", "en"], platform: "Linux x86_64",
                 onLine: true, cookieEnabled: true, globalPrivacyControl: true, doNotTrack: "1", hardwareConcurrency: 2, maxTouchPoints: 0, vendor: "" },
    screen: { width: 1366, height: 768, availWidth: 1366, availHeight: 768, colorDepth: 24 },
    devicePixelRatio: 1, scrollX: 0, pageXOffset: 0,
    scrollTo(a, b) { __scrollTo(typeof a === "object" && a ? Number(a.top || 0) : Number(b || 0)); },
    scrollBy(a, b) { __scrollTo(g.scrollY + (typeof a === "object" && a ? Number(a.top || 0) : Number(b || 0))); },
    scroll(a, b) { g.scrollTo(a, b); },
    focus() {}, blur() {}, print() {}, open() { return null; }, close() {},
    alert: (m) => __log("alert", String(m)), confirm: () => false, prompt: () => null,
    getComputedStyle: (el) => el.style,
    matchMedia: (q) => {
      // Width, height, orientation and preference queries, joined with "and" / ",".
      const test = (part) => {
        part = part.trim().toLowerCase();
        if (!part || part === "all" || part === "screen") return true;
        if (part === "print" || part === "speech") return false;
        const m = /^\(\s*([a-z-]+)\s*(?::\s*([^)]+))?\)$/.exec(part);
        if (!m) return /^only\s+screen$/.test(part);
        const [, f, raw] = m, v = (raw || "").trim();
        const px = parseFloat(v) * (/em$/.test(v) ? 16 : 1);
        switch (f) {
          case "min-width": return g.innerWidth >= px;
          case "max-width": return g.innerWidth <= px;
          case "min-height": return g.innerHeight >= px;
          case "max-height": return g.innerHeight <= px;
          case "orientation": return v === (g.innerWidth >= g.innerHeight ? "landscape" : "portrait");
          case "prefers-color-scheme": return v === "light";
          case "prefers-reduced-motion": return v === "no-preference";
          case "hover": case "any-hover": return v === "hover";
          case "pointer": case "any-pointer": return v === "fine";
          case "color": return true;
          default: return false;
        }
      };
      const matches = String(q).split(",").some((alt) => {
        const not = /^\s*not\s+/i.test(alt);
        const r = alt.replace(/^\s*(not|only)\s+/i, "").split(/\s+and\s+/i).every(test);
        return not ? !r : r;
      });
      return { matches, media: q, onchange: null, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent() { return true; } };
    },
    localStorage: storage(true), sessionStorage: storage(false),
    history,
    performance: { now: () => __nowMs, mark() {}, measure() {}, getEntriesByType: () => [], timing: {} },
    console: { log: consoleFn("log"), info: consoleFn("info"), debug: consoleFn("debug"),
               warn: consoleFn("warn"), error: consoleFn("error"), trace: consoleFn("trace"),
               dir: consoleFn("log"), table: consoleFn("log"), group() {}, groupEnd() {}, time() {}, timeEnd() {} },
    MutationObserver: class { observe() {} disconnect() {} takeRecords() { return []; } },
    IntersectionObserver: class { observe() {} unobserve() {} disconnect() {} },
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    CSS: { supports: () => false, escape: (s) => String(s).replace(/([^\w-])/g, "\\$1") },
  });
  // Live window geometry from the layout.
  const vp = () => __viewport().split(",").map(Number);
  for (const [name, i] of [["innerWidth", 0], ["outerWidth", 0], ["innerHeight", 1], ["scrollY", 2], ["pageYOffset", 2]]) {
    Object.defineProperty(g, name, { get: () => (i === 2 ? vp()[2] : Math.round(vp()[i])), configurable: true });
  }
  Object.defineProperty(g, "outerHeight", { get: () => Math.round(vp()[1]) + 80, configurable: true });
})();
