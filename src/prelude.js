// Ironswift DOM bindings.
// The Rust side exposes a handful of primitives (__tag, __children, __append, ...)
// that work on node ids. Everything web pages see (document, Element, events,
// timers...) is built here in JavaScript on top of them.
(function () {
  "use strict";
  const g = globalThis;
  // Element objects by node id (a plain array: the native list functions read it directly).
  const cache = [];
  globalThis.__wrapCache = cache;

  // Node objects are made natively (dom_classes.rs) once the classes below are installed;
  // slowWrap makes the kinds that stay JavaScript objects (the document, shadow roots).
  let nativeWrap = null;
  function wrap(id) {
    if (id === undefined || id === null || id < 0) return null;
    return cache[id] || (nativeWrap ? nativeWrap(id) : slowWrap(id));
  }
  function slowWrap(id) {
    let n = cache[id];
    if (n) return n;
    const t = __type(id);
    n = t === 9 ? document : t === 3 ? new Text(id) : t === 8 ? new Comment(id) : t === 11 && __shadowHost(id) ? new ShadowRoot(id, SHADOW_KEY) : t === 11 ? new DocumentFragment(id) : new Element(id);
    cache[id] = n;
    return n;
  }
  globalThis.__wrapFn = wrap;
  const idOf = (n) => (n && n.__id !== undefined ? n.__id : -1);
  // ASCII-lowercase on HTML elements. Most names have no capitals at all: skip the regex then
  // (this runs on every getAttribute/setAttribute, className included).
  const attrName = (el, n) => {
    n = String(n);
    if (n === n.toLowerCase() || el.namespaceURI !== "http://www.w3.org/1999/xhtml") return n;
    return n.replace(/[A-Z]+/g, (c) => c.toLowerCase());
  };
  // Valid attribute names seen so far, so the check's regex runs once per name.
  const validAttrNames = new Set();
  const checkAttrName = (n) => {
    if (validAttrNames.has(n)) return;
    if (!/^[^\t\n\f\r \0\/=>]+$/.test(n)) throw new DOMException("'" + n + "' is not a valid attribute name.", "InvalidCharacterError");
    if (validAttrNames.size < 4096) validAttrNames.add(n);
  };
  const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());

  // ------------------------------------------------------------ events
  class Event {
    constructor(type, init = {}) {
      this.type = String(type);
      this.bubbles = !!init.bubbles;
      this.cancelable = !!init.cancelable;
      this.composed = !!init.composed;
      this.defaultPrevented = false;
      this.target = null;
      this.currentTarget = null;
      // Only events that have one (mouse, focus…) carry relatedTarget.
      if (init && "relatedTarget" in init) this.relatedTarget = init.relatedTarget ?? null;
      this.eventPhase = 0;
      this.isTrusted = false;
      this.timeStamp = __nowMs;
      this._stop = false;
      this._stopNow = false;
      this._path = [];
    }
    static get NONE() { return 0; }
    static get CAPTURING_PHASE() { return 1; }
    static get AT_TARGET() { return 2; }
    static get BUBBLING_PHASE() { return 3; }
    get srcElement() { return this.target; }
    get returnValue() { return !this.defaultPrevented; }
    set returnValue(v) { if (!v) this.preventDefault(); }
    get cancelBubble() { return this._stop; }
    set cancelBubble(v) { if (v) this._stop = true; }
    preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
    stopPropagation() { this._stop = true; }
    stopImmediatePropagation() { this._stop = true; this._stopNow = true; }
    initEvent(type, bubbles, cancelable) { this.type = String(type); this.bubbles = !!bubbles; this.cancelable = !!cancelable; }
    // The event's path as seen from the current listener: closed shadow trees the current
    // target isn't in stay hidden (DOM standard algorithm).
    composedPath() {
      const path = this._path, cur = this.currentTarget;
      if (!path.length || !cur) return [];
      const out = [cur];
      let curIndex = 0, level = 0;
      for (let i = path.length - 1; i >= 0; i--) {
        if (path[i].rootOfClosedTree) level++;
        if (path[i].item === cur) { curIndex = i; break; }
        if (path[i].slotInClosedTree) level--;
      }
      let current = level, max = level;
      for (let i = curIndex - 1; i >= 0; i--) {
        if (path[i].rootOfClosedTree) current++;
        if (current <= max) out.unshift(path[i].item);
        if (path[i].slotInClosedTree) { current--; if (current < max) max = current; }
      }
      current = max = level;
      for (let i = curIndex + 1; i < path.length; i++) {
        if (path[i].slotInClosedTree) current++;
        if (current <= max) out.push(path[i].item);
        if (path[i].rootOfClosedTree) { current--; if (current < max) max = current; }
      }
      return out;
    }
  }
  class CustomEvent extends Event {
    constructor(type, init = {}) { super(type, init); this.detail = init.detail ?? null; }
  }
  class UIEvent extends Event {
    constructor(type, init = {}) { super(type, init); this.view = init.view ?? null; this.detail = init.detail ?? 0; }
  }
  class MouseEvent extends UIEvent {
    constructor(type, init = {}) {
      super(type, init);
      for (const k of ["screenX", "screenY", "clientX", "clientY", "button", "buttons", "movementX", "movementY"]) this[k] = init[k] ?? 0;
      for (const k of ["ctrlKey", "shiftKey", "altKey", "metaKey"]) this[k] = !!init[k];
      this.relatedTarget = init.relatedTarget ?? null;
    }
    get pageX() { return this.clientX; }
    get pageY() { return this.clientY; }
    get x() { return this.clientX; }
    get y() { return this.clientY; }
    get offsetX() { const r = this.target && this.target.getBoundingClientRect ? this.target.getBoundingClientRect() : { left: 0 }; return this.clientX - r.left; }
    get offsetY() { const r = this.target && this.target.getBoundingClientRect ? this.target.getBoundingClientRect() : { top: 0 }; return this.clientY - r.top; }
    getModifierState(k) { return { Control: this.ctrlKey, Shift: this.shiftKey, Alt: this.altKey, Meta: this.metaKey }[k] || false; }
  }
  class PointerEvent extends MouseEvent {
    constructor(type, init = {}) {
      super(type, init);
      this.pointerId = init.pointerId ?? 0;
      this.pointerType = init.pointerType ?? "";
      this.isPrimary = !!init.isPrimary;
      this.width = init.width ?? 1;
      this.height = init.height ?? 1;
      this.pressure = init.pressure ?? 0;
    }
    getCoalescedEvents() { return []; }
    getPredictedEvents() { return []; }
  }
  class WheelEvent extends MouseEvent {
    constructor(type, init = {}) { super(type, init); this.deltaX = init.deltaX ?? 0; this.deltaY = init.deltaY ?? 0; this.deltaZ = init.deltaZ ?? 0; this.deltaMode = init.deltaMode ?? 0; }
  }
  class FocusEvent extends UIEvent {
    constructor(type, init = {}) { super(type, init); this.relatedTarget = init.relatedTarget ?? null; }
  }
  class KeyboardEvent extends UIEvent {
    constructor(type, init = {}) {
      super(type, init);
      this.key = init.key ?? "";
      this.code = init.code ?? "";
      this.location = init.location ?? 0;
      this.repeat = !!init.repeat;
      for (const k of ["ctrlKey", "shiftKey", "altKey", "metaKey"]) this[k] = !!init[k];
    }
    getModifierState(k) { return { Control: this.ctrlKey, Shift: this.shiftKey, Alt: this.altKey, Meta: this.metaKey }[k] || false; }
  }
  for (const [name, C] of Object.entries({ UIEvent, MouseEvent, PointerEvent, WheelEvent, FocusEvent, KeyboardEvent })) {
    if (!g[name]) g[name] = C;
  }
  const isShadowRoot = (n) => n instanceof ShadowRoot;
  // The root of a node's tree (a document, a shadow root, or a detached subtree's top).
  const rootOf = (n) => { while (n && n.parentNode) n = n.parentNode; return n; };
  // Is `a` an ancestor of (or equal to) `b`, crossing from shadow roots to their hosts?
  const shadowIncludingAncestor = (a, b) => {
    while (b) {
      if (b === a) return true;
      b = b.parentNode || (isShadowRoot(b) ? b.host : null);
    }
    return false;
  };
  // A node as seen from `other`: moved out to shadow hosts until it's in a tree `other` can see.
  const retarget = (a, b) => {
    while (a && a.nodeType !== undefined) {
      const root = rootOf(a);
      if (!isShadowRoot(root) || shadowIncludingAncestor(root, b)) return a;
      a = root.host;
    }
    return a;
  };
  // "Get the parent" for event paths.
  function eventParent(node, ev, first) {
    if (node === g) return null;
    if (node === document) return ev.type === "load" ? null : g;
    if (isShadowRoot(node)) return !ev.composed && rootOf(first) === node ? null : node.host;
    if (node.nodeType !== undefined && node.assignedSlot !== undefined) {
      const slot = __assignedSlotAny(node);
      if (slot) return slot;
    }
    return node.parentNode || null;
  }
  class EventTarget {
    addEventListener(type, fn, opts) {
      if (!fn) return;
      const capture = typeof opts === "boolean" ? opts : !!(opts && opts.capture);
      const once = !!(opts && typeof opts === "object" && opts.once);
      const l = (this.__listeners ||= {});
      const list = (l[type] ||= []);
      if (list.some((x) => x.fn === fn && x.capture === capture)) return;
      list.push({ fn, once, capture });
    }
    removeEventListener(type, fn, opts) {
      const capture = typeof opts === "boolean" ? opts : !!(opts && opts.capture);
      const l = this.__listeners && this.__listeners[type];
      if (l) {
        for (const x of l) if (x.fn === fn && x.capture === capture) x.removed = true;
        this.__listeners[type] = l.filter((x) => !(x.fn === fn && x.capture === capture));
      }
    }
    // Run this target's listeners: phase "capture" runs capturing ones, "bubble" the rest
    // (and the inline on… handler); undefined runs all of them.
    __fire(ev, phase) {
      ev.currentTarget = this;
      const list = (this.__listeners && this.__listeners[ev.type]) || [];
      for (const entry of list.slice()) {
        if (entry.removed) continue;
        if (phase === "capture" && !entry.capture) continue;
        if (phase === "bubble" && entry.capture) continue;
        if (entry.once) this.removeEventListener(ev.type, entry.fn, { capture: entry.capture });
        try {
          typeof entry.fn === "function" ? entry.fn.call(this, ev) : entry.fn.handleEvent(ev);
        } catch (e) { console.error(String(e)); if (g.__onListenerError) g.__onListenerError(e); }
        if (ev._stopNow) return;
      }
      if (phase === "capture") return;
      const prop = this["on" + ev.type];
      if (typeof prop === "function") {
        try { if (prop.call(this, ev) === false) ev.preventDefault(); } catch (e) { console.error(String(e)); if (g.__onListenerError) g.__onListenerError(e); }
      } else if (this.getAttribute && this.getAttribute("on" + ev.type)) {
        try {
          const code = this.getAttribute("on" + ev.type);
          if (!__cspInline("attr", code)) return;
          const f = new Function("event", code);
          if (f.call(this, ev) === false) ev.preventDefault();
        } catch (e) { console.error(String(e)); }
      }
    }
    dispatchEvent(ev) {
      const target = this;
      const related = ev.relatedTarget;
      const path = [];
      for (let node = target; node; node = eventParent(node, ev, target)) {
        const closedRoot = isShadowRoot(node) && node.mode === "closed";
        let slotInClosed = false;
        if (node.nodeType === 1 && node.localName === "slot") {
          const r = rootOf(node);
          slotInClosed = isShadowRoot(r) && r.mode === "closed";
        }
        path.push({ item: node, rootOfClosedTree: closedRoot, slotInClosedTree: slotInClosed });
        if (path.length > 10000) break;
      }
      ev._path = path;
      ev._stop = false;
      ev._stopNow = false;
      const seenFrom = (item) => (item === g ? retarget(target, document) : retarget(target, item));
      // Capture: outermost first.
      for (let i = path.length - 1; i >= 0 && !ev._stop; i--) {
        const item = path[i].item;
        ev.target = seenFrom(item);
        if (related) ev.relatedTarget = retarget(related, item);
        ev.eventPhase = ev.target === item ? 2 : 1;
        item.__fire(ev, "capture");
      }
      // Target and bubble: innermost first.
      for (let i = 0; i < path.length && !ev._stop; i++) {
        const item = path[i].item;
        ev.target = seenFrom(item);
        if (related) ev.relatedTarget = retarget(related, item);
        const atTarget = ev.target === item;
        if (!atTarget && !ev.bubbles) continue;
        ev.eventPhase = atTarget ? 2 : 3;
        item.__fire(ev, "bubble");
      }
      ev.eventPhase = 0;
      ev.currentTarget = null;
      ev.target = retarget(target, document);
      if (related) ev.relatedTarget = retarget(related, document);
      ev._path = [];
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
    get childNodes() { return __wrapList(__children(this.__id)); }
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
    get tagName() { const t = __tag(this.__id); return this.namespaceURI === "http://www.w3.org/1999/xhtml" ? t.toUpperCase() : t; }
    get nodeName() { return this.tagName; }
    get localName() { return __tag(this.__id); }
    // Attribute names are ASCII-lowercased on HTML elements only.
    getAttribute(n) { return __getAttr(this.__id, attrName(this, n)); }
    setAttribute(n, v) {
      n = String(n);
      checkAttrName(n);
      __setAttr(this.__id, attrName(this, n), String(v));
    }
    removeAttribute(n) { __removeAttr(this.__id, attrName(this, n)); }
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
    attachShadow(init) {
      const mode = init && init.mode;
      if (mode !== "open" && mode !== "closed") throw new TypeError("attachShadow: mode must be 'open' or 'closed'");
      if (__shadowHost(__shadowRoot(this.__id)) || this.__hasShadow) throw new DOMException("This element already has a shadow root", "NotSupportedError");
      this.__hasShadow = true;
      const root = wrap(__attachShadow(this.__id, mode === "open"));
      root.__manual = init.slotAssignment === "manual";
      root.__delegatesFocus = !!init.delegatesFocus;
      root.__clonable = !!init.clonable;
      root.__serializable = !!init.serializable;
      return root;
    }
    get shadowRoot() { return wrap(__shadowRoot(this.__id)); }
    get content() { return this.localName === "template" ? wrap(__templateContent(this.__id)) : undefined; }
    getRootNode(opts) {
      let n = this;
      for (;;) {
        while (n.parentNode) n = n.parentNode;
        if (opts && opts.composed && n instanceof ShadowRoot) { n = n.host; continue; }
        return n;
      }
    }
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
    querySelectorAll(s) { return __wrapList(__query(this.__id, String(s), true)); }
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
    click() { this.dispatchEvent(new Event("click", { bubbles: true, cancelable: true, composed: true })); }
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

  const SHADOW_KEY = Symbol("shadow");
  class ShadowRoot extends DocumentFragment {
    constructor(id, key) {
      if (key !== SHADOW_KEY) throw new TypeError("Illegal constructor");
      super(id);
    }
    get slotAssignment() { return (this.__manual ? "manual" : "named"); }
    get delegatesFocus() { return !!this.__delegatesFocus; }
    get clonable() { return !!this.__clonable; }
    get serializable() { return !!this.__serializable; }
    get nodeName() { return "#document-fragment"; }
    get host() { return wrap(__shadowHost(this.__id)[0]); }
    get mode() { return __shadowHost(this.__id)[1] ? "open" : "closed"; }
    get innerHTML() { return __getHTML(this.__id, false); }
    set innerHTML(v) { __setHTML(this.__id, String(v)); }
    get children() { return this.childNodes.filter((n) => n.nodeType === 1); }
    getElementById(id) { return this.querySelector("#" + CSS.escape(id)); }
    get activeElement() { return null; }
  }
  g.ShadowRoot = ShadowRoot;

  // ------------------------------------------------------------ slots
  const slotNameOf = (n) => (n.nodeType === 1 ? n.getAttribute("slot") || "" : n.nodeType === 3 ? "" : null);
  // The slot a host's child is assigned to (in an open or closed shadow tree), or null.
  function __assignedSlotAny(node) {
    const host = node.parentNode;
    if (!host || host.nodeType !== 1) return null;
    const root = wrap(__shadowRoot(host.__id, true));
    if (!root) return null;
    if (root.__manual) {
      // Manually assigned: the slot must still be in this host's shadow tree.
      const s = node.__manualSlot;
      return s && rootOf(s) === root ? s : null;
    }
    const name = slotNameOf(node);
    if (name === null) return null;
    for (const s of Element.prototype.querySelectorAll.call(root, "slot")) if ((s.getAttribute("name") || "") === name) return s;
    return null;
  }
  g.__assignedSlotAny = __assignedSlotAny;
  function assignedTo(slot) {
    const r = rootOf(slot);
    if (!isShadowRoot(r)) return [];
    return r.host.childNodes.filter((n) => __assignedSlotAny(n) === slot);
  }
  // Manual slot assignment (attachShadow({slotAssignment: "manual"}) + slot.assign(...)).
  Element.prototype.assign = function (...nodes) {
    const list = [];
    for (const n of nodes) if (!list.includes(n)) list.push(n);
    // A node can be manually assigned to only one slot.
    for (const n of list) if (n.__manualSlot && n.__manualSlot !== this) n.__manualSlot.__manual = (n.__manualSlot.__manual || []).filter((x) => x !== n);
    for (const old of this.__manual || []) if (!list.includes(old)) delete old.__manualSlot;
    for (const n of list) n.__manualSlot = this;
    this.__manual = list;
    g.__slotsMaybeChanged();
  };
  function flattenedOf(slot) {
    if (!isShadowRoot(rootOf(slot))) return [];
    let list = assignedTo(slot);
    if (!list.length) list = slot.childNodes.filter((n) => n.nodeType === 1 || n.nodeType === 3);
    const out = [];
    for (const n of list) {
      if (n.nodeType === 1 && n.localName === "slot" && isShadowRoot(rootOf(n))) out.push(...flattenedOf(n));
      else out.push(n);
    }
    return out;
  }
  Object.defineProperty(Node.prototype, "assignedSlot", {
    configurable: true,
    get() {
      const s = __assignedSlotAny(this);
      return s && rootOf(s).mode === "open" ? s : null;
    },
  });
  Object.assign(Element.prototype, {
    assignedNodes(opts) { return opts && opts.flatten ? flattenedOf(this) : assignedTo(this); },
    assignedElements(opts) { return this.assignedNodes(opts).filter((n) => n.nodeType === 1); },
  });
  Object.defineProperty(Element.prototype, "slot", {
    configurable: true,
    get() { return this.getAttribute("slot") || ""; },
    set(v) { this.setAttribute("slot", String(v)); },
  });

  class Document extends Node {
    get nodeName() { return "#document"; }
    get documentElement() { return this.querySelector("html"); }
    get head() { return this.querySelector("head"); }
    get body() { return this.querySelector("body"); }
    set body(v) {
      // HTML: replaces the body (or adds one) under <html>.
      if (!v || v.nodeType !== 1 || (v.localName !== "body" && v.localName !== "frameset")) throw new DOMException("The new body element must be a 'body' or 'frameset' element.", "HierarchyRequestError");
      const old = this.body, root = this.documentElement;
      if (old === v) return;
      if (old) old.parentNode.replaceChild(v, old);
      else if (root) root.appendChild(v);
      else throw new DOMException("No document element.", "HierarchyRequestError");
    }
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
    createElement(t) {
      t = String(t);
      if (!__validName(t)) throw new DOMException("'" + t + "' is not a valid element name.", "InvalidCharacterError");
      return wrap(__create(t === t.toLowerCase() ? t : t.replace(/[A-Z]+/g, (c) => c.toLowerCase())));
    }
    createElementNS(ns, t) {
      t = String(t);
      // HTML custom element names go through createElement, which upgrades them.
      if (ns === "http://www.w3.org/1999/xhtml" && t.includes("-") && !/[:A-Z]/.test(t) && __validName(t)) return this.createElement(t);
      const r = __createNS(ns, t);
      if (typeof r === "string") throw new DOMException("createElementNS: '" + t + "' is not a valid qualified name here.", r);
      return wrap(r);
    }
    createTextNode(s) { return wrap(__createText(String(s))); }
    createComment() { return wrap(__createText("")); }
    createDocumentFragment() { const f = new DocumentFragment(__create("#fragment")); cache[f.__id] = f; return f; }
    createEvent() { return new Event(""); }
    getElementById(id) { return wrap(__byId(String(id))); }
    querySelector(s) { return wrap(__query(0, String(s), false)[0]); }
    querySelectorAll(s) { return __wrapList(__query(0, String(s), true)); }
    getElementsByTagName(t) { return Element.prototype.getElementsByTagName.call(this, t); }
    getElementsByClassName(c) { return Element.prototype.getElementsByClassName.call(this, c); }
    getElementsByName(n) { return this.querySelectorAll('[name="' + n + '"]'); }
    write(...html) { __write(html.join("")); }
    writeln(...html) { __write(html.join("") + "\n"); }
    hasFocus() { return true; }
  }

  const document = new Document(0);
  cache[0] = document;

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
  // The same, counting repeating timers too (a worker sleeps until then).
  g.__nextTimerAny = () => { let m = -1; for (const t of timers.values()) if (m < 0 || t.due < m) m = t.due; return m < 0 ? -1 : Math.max(0, m - __nowMs); };

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
  // Bare calls (`addEventListener(...)` with no `window.`) have no `this` in strict code:
  // bind the window's own event methods to the window.
  for (const m of ["addEventListener", "removeEventListener", "dispatchEvent"]) {
    const f = EventTarget.prototype[m];
    Object.defineProperty(g, m, { value: function (...a) { return f.apply(this == null ? g : this, a); }, writable: true, configurable: true });
  }
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
    // Errors have no enumerable fields (JSON gives "{}"): show "Name: message" and where.
    if (x instanceof Error) return String(x) + (x.stack ? "\n" + String(x.stack).split("\n").slice(0, 4).join("\n") : "");
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

  // A body is held as text or as bytes (whichever it arrived as) and converted on demand:
  // text() decodes bytes as UTF-8, arrayBuffer()/blob() encode text as UTF-8.
  const bodyBytes = (b) => {
    if (b instanceof ArrayBuffer) return new Uint8Array(b.slice(0));
    if (ArrayBuffer.isView(b)) return new Uint8Array(b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength));
    if (typeof g.Blob === "function" && b instanceof g.Blob) return b.__bytes().slice();
    return null;
  };
  const asText = (o) => (o._text !== null ? o._text : new g.TextDecoder().decode(o._bytes));
  const asBytes = (o) => (o._bytes !== null ? o._bytes : new g.TextEncoder().encode(o._text));
  class Response {
    constructor(body, init) {
      init = init || {};
      this.status = init.status === undefined ? 200 : init.status;
      this.statusText = init.statusText || "";
      this.headers = new Headers(init.headers);
      this.url = init.url || "";
      this.type = init.type || "basic";
      this.redirected = !!init.redirected;
      this.bodyUsed = false;
      this._text = ""; this._bytes = null;
      if (body == null) return;
      const bytes = bodyBytes(body);
      if (bytes) {
        this._text = null; this._bytes = bytes;
        if (body.type && !this.headers.has("content-type")) this.headers.set("content-type", body.type);
      } else this._text = bodyString(body, this.headers);
    }
    get ok() { return this.status >= 200 && this.status < 300; }
    _take() {
      if (this.bodyUsed) return Promise.reject(new TypeError("Body has already been consumed."));
      this.bodyUsed = true;
      return Promise.resolve(this);
    }
    text() { return this._take().then(asText); }
    json() { return this._take().then((o) => JSON.parse(asText(o))); }
    arrayBuffer() { return this._take().then((o) => asBytes(o).slice().buffer); }
    bytes() { return this._take().then((o) => asBytes(o).slice()); }
    blob() { return this._take().then((o) => new g.Blob([asBytes(o)], { type: this.headers.get("content-type") || "" })); }
    formData() { return this._take().then((o) => { const f = new FormData(); for (const [k, v] of new URLSearchParams(asText(o))) f.append(k, v); return f; }); }
    clone() {
      if (this.bodyUsed) throw new TypeError("Body has already been consumed.");
      const r = new Response(null, this);
      r.url = this.url;
      r._text = this._text; r._bytes = this._bytes;
      return r;
    }
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
  // Legacy numeric codes, still checked by scripts and tests.
  const DOM_CODES = { IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4, InvalidCharacterError: 5, NoModificationAllowedError: 7, NotFoundError: 8, NotSupportedError: 9, InUseAttributeError: 10, InvalidStateError: 11, SyntaxError: 12, InvalidModificationError: 13, NamespaceError: 14, InvalidAccessError: 15, TypeMismatchError: 17, SecurityError: 18, NetworkError: 19, AbortError: 20, URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23, InvalidNodeTypeError: 24, DataCloneError: 25 };
  class DOMException extends Error {
    constructor(message, name) { super(message === undefined ? "" : String(message)); this.name = name === undefined ? "Error" : String(name); }
    get code() { return DOM_CODES[this.name] || 0; }
  }
  const CODE_NAMES = { INDEX_SIZE_ERR: 1, DOMSTRING_SIZE_ERR: 2, HIERARCHY_REQUEST_ERR: 3, WRONG_DOCUMENT_ERR: 4, INVALID_CHARACTER_ERR: 5, NO_DATA_ALLOWED_ERR: 6, NO_MODIFICATION_ALLOWED_ERR: 7, NOT_FOUND_ERR: 8, NOT_SUPPORTED_ERR: 9, INUSE_ATTRIBUTE_ERR: 10, INVALID_STATE_ERR: 11, SYNTAX_ERR: 12, INVALID_MODIFICATION_ERR: 13, NAMESPACE_ERR: 14, INVALID_ACCESS_ERR: 15, VALIDATION_ERR: 16, TYPE_MISMATCH_ERR: 17, SECURITY_ERR: 18, NETWORK_ERR: 19, ABORT_ERR: 20, URL_MISMATCH_ERR: 21, QUOTA_EXCEEDED_ERR: 22, TIMEOUT_ERR: 23, INVALID_NODE_TYPE_ERR: 24, DATA_CLONE_ERR: 25 };
  for (const [k, v] of Object.entries(CODE_NAMES)) {
    Object.defineProperty(DOMException, k, { value: v });
    Object.defineProperty(DOMException.prototype, k, { value: v });
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
    return String(b);
  };
  // What goes to __netStart: a string, or a Uint8Array for bytes (sent unchanged).
  const requestBody = (b, headers) => {
    const bytes = b == null ? null : bodyBytes(b);
    if (!bytes) return bodyString(b, headers);
    if (b.type && !headers.has("content-type")) headers.set("content-type", b.type);
    return bytes;
  };
  // A finished request's body, as the text or bytes it came in as.
  const takeBody = (r) => (r.bin ? { _text: null, _bytes: new Uint8Array(__netBody(r.bin)) } : { _text: r.body == null ? "" : r.body, _bytes: null });

  g.fetch = function (input, init) {
    let req;
    try { req = new Request(input, init); } catch (e) { return Promise.reject(e); }
    return new Promise((resolve, reject) => {
      if (req.signal && req.signal.aborted) return reject(req.signal.reason);
      const id = nextNet++;
      const body = requestBody(req._body, req.headers);
      pending.set(id, (r) => {
        if (r.error) { console.warn("fetch failed: " + r.error); return reject(new TypeError("Failed to fetch")); }
        const res = new Response(null, { status: r.status, statusText: r.statusText, headers: r.headers, url: r.url });
        Object.assign(res, takeBody(r));
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
      if (this.responseType === "arraybuffer" || this.responseType === "blob") {
        if (this.readyState !== 4 || !this._body) return null;
        const b = asBytes(this._body);
        return this.responseType === "blob" ? new g.Blob([b], { type: this._resHeaders.get("content-type") || "" }) : b.slice().buffer;
      }
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
      this._body = takeBody(r);
      this.responseText = this.responseType === "arraybuffer" || this.responseType === "blob" ? "" : asText(this._body);
      this._state(3);
      this._state(4);
      this._fire("load");
      this._fire("loadend");
    }
    send(body) {
      if (this.readyState !== 1) throw new DOMException("The object's state must be OPENED.", "InvalidStateError");
      const b = requestBody(body, this._headers);
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
  const isHtml = (x) => isEl(x) && x.namespaceURI === "http://www.w3.org/1999/xhtml";
  g.__isHtmlElement = isHtml;
  // Every element the HTML spec defines (others in the HTML namespace are HTMLUnknownElement,
  // unless their name is a valid custom element name).
  const KNOWN_HTML = new Set((Object.values(interfaces).join(" ") + " abbr acronym address article aside b basefont bdi bdo big center cite code dd dfn dir dt em figcaption figure font footer frame frameset header hgroup i ins del kbd main mark marquee menu nav noembed noframes nobr noscript param plaintext rb rp rt rtc ruby s samp search section small strike strong sub summary sup tt u var wbr xmp search slot").split(" "));
  for (const [name, tags] of Object.entries(interfaces)) {
    const set = new Set(tags.split(" "));
    const C = class extends HTMLElement { static [Symbol.hasInstance](x) { return isHtml(x) && set.has(x.localName); } };
    Object.defineProperty(C, "name", { value: name });
    g[name] = C;
  }
  // Legacy factories: new Image(w, h) and new Option(text, value, defaultSelected, selected).
  if (typeof g.Image !== "function") {
    g.Image = function Image(w, h) {
      const img = document.createElement("img");
      if (w !== undefined) img.setAttribute("width", String(w >>> 0));
      if (h !== undefined) img.setAttribute("height", String(h >>> 0));
      return img;
    };
    g.Image.prototype = g.HTMLImageElement.prototype;
  }
  if (typeof g.Option !== "function") {
    g.Option = function Option(text, value, defaultSelected, selected) {
      const o = document.createElement("option");
      if (text !== undefined) o.textContent = String(text);
      if (value !== undefined) o.setAttribute("value", String(value));
      if (defaultSelected) o.setAttribute("selected", "");
      if (selected) o.selected = true;
      return o;
    };
    g.Option.prototype = g.HTMLOptionElement.prototype;
  }
  const isSvg = (x) => isEl(x) && x.namespaceURI === "http://www.w3.org/2000/svg";
  // The hot read-only DOM members as native functions (dom_classes.rs), replacing the
  // JavaScript ones above; setters and everything that changes the tree stay JavaScript.
  try {
    const nat = __domInstall({ Element, Text, Comment, DocumentFragment, cache, slow: slowWrap });
    const put = (P, m) => {
      for (const [k, get] of Object.entries(m.get)) {
        const d = Object.getOwnPropertyDescriptor(P, k);
        Object.defineProperty(P, k, { get, set: d && d.set, enumerable: false, configurable: true });
      }
      for (const [k, value] of Object.entries(m.fn)) Object.defineProperty(P, k, { value, writable: true, enumerable: false, configurable: true });
    };
    const classes = { Node, Element, CharacterData, Text, Document };
    for (const [name, m] of Object.entries(nat.interfaces)) if (classes[name]) put(classes[name].prototype, m);
    nativeWrap = nat.wrap;
  } catch (e) { console.error("native DOM setup failed: " + e); }
  // Fragments and shadow roots hold children like elements do: give them the same tree methods.
  for (const name of ["appendChild", "insertBefore", "removeChild", "replaceChild", "childNodes", "firstChild", "lastChild", "children",
    "firstElementChild", "lastElementChild", "childElementCount", "textContent", "cloneNode", "contains", "hasChildNodes", "append", "prepend",
    "replaceChildren", "querySelector", "querySelectorAll", "getElementsByTagName", "getElementsByClassName", "normalize"]) {
    const d = Object.getOwnPropertyDescriptor(Element.prototype, name) || Object.getOwnPropertyDescriptor(Node.prototype, name);
    if (d && !Object.prototype.hasOwnProperty.call(DocumentFragment.prototype, name)) Object.defineProperty(DocumentFragment.prototype, name, d);
  }
  // Every node (text, comment, document…) has getRootNode, not just elements.
  if (!Object.prototype.hasOwnProperty.call(Node.prototype, "getRootNode")) Node.prototype.getRootNode = Element.prototype.getRootNode;
  if (!DocumentFragment.prototype.getElementById) DocumentFragment.prototype.getElementById = function (id) { return this.querySelector("#" + CSS.escape(String(id))); };

  // slotchange: after DOM changes, once the current task's microtasks run, every slot whose
  // assigned nodes changed gets a slotchange event (bubbles, not composed).
  const slotSeen = new WeakMap();
  let slotCheckQueued = false;
  const sameList = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);
  function checkSlots() {
    slotCheckQueued = false;
    const changed = new Set();
    for (const r of __shadowRootList()) {
      const root = wrap(r);
      if (!root) continue;
      for (const slot of Element.prototype.querySelectorAll.call(root, "slot")) {
        const now = assignedTo(slot);
        const before = slotSeen.get(slot) || [];
        slotSeen.set(slot, now);
        if (!sameList(now, before)) changed.add(slot);
      }
    }
    // A slot that is itself slotted into another slot passes the change on to that one.
    for (const slot of [...changed]) {
      let s = __assignedSlotAny(slot);
      while (s && !changed.has(s)) {
        changed.add(s);
        s = __assignedSlotAny(s);
      }
    }
    for (const slot of changed) slot.dispatchEvent(new Event("slotchange", { bubbles: true }));
  }
  g.__slotsMaybeChanged = function () {
    if (slotCheckQueued || !__shadowRootList().length) return;
    slotCheckQueued = true;
    Promise.resolve().then(checkSlots);
  };
  for (const name of ["__setText", "__insert", "__detach", "__setAttr", "__removeAttr", "__setHTML", "__attachShadow"]) {
    const orig = g[name];
    if (typeof orig !== "function") continue;
    g[name] = function (...args) {
      const r = orig.apply(this, args);
      g.__slotsMaybeChanged();
      return r;
    };
  }

  // MutationObserver: the natives that change the tree are wrapped to record what
  // changed; records are delivered to matching observers in a microtask.
  {
    const regs = []; // { obs, target (id), opts }
    const pending = new Set();
    let queued = false;
    const ancestors = (id) => { const out = []; let n = id, guard = 0; while (n >= 0 && guard++ < 10000) { out.push(n); n = __parent(n); } return out; };
    const deliver = () => {
      queued = false;
      const list = [...pending];
      pending.clear();
      for (const o of list) {
        const recs = o.__records.splice(0);
        if (recs.length) { try { o.__cb.call(o, recs, o); } catch (e) { __log("error", "MutationObserver callback: " + e); } }
      }
    };
    const record = (type, targetId, extra) => {
      if (!regs.length) return;
      const chain = ancestors(targetId);
      const seen = new Set();
      for (const r of regs) {
        if (seen.has(r.obs)) continue;
        const depth = chain.indexOf(r.target);
        if (depth < 0 || (depth > 0 && !r.opts.subtree)) continue;
        const o = r.opts;
        if (type === "childList" && !o.childList) continue;
        if (type === "attributes" && (!o.attributes || (o.attributeFilter && !o.attributeFilter.includes(extra.attributeName)))) continue;
        if (type === "characterData" && !o.characterData) continue;
        seen.add(r.obs);
        const rec = {
          type, target: wrap(targetId),
          addedNodes: extra.added || [], removedNodes: extra.removed || [],
          previousSibling: extra.prev || null, nextSibling: extra.next || null,
          attributeName: extra.attributeName == null ? null : extra.attributeName, attributeNamespace: null,
          oldValue: (type === "attributes" && o.attributeOldValue) || (type === "characterData" && o.characterDataOldValue) ? extra.oldValue : null,
        };
        r.obs.__records.push(rec);
        pending.add(r.obs);
      }
      if (pending.size && !queued) { queued = true; Promise.resolve().then(deliver); }
    };
    const kids = (id) => Array.from(__children(id) || []);
    const hook = (name, fn) => {
      const orig = g[name];
      if (typeof orig !== "function") return;
      g[name] = function (...args) { return regs.length ? fn(orig, ...args) : orig.apply(this, args); };
    };
    hook("__insert", (orig, p, c, b) => {
      const old = __parent(c);
      if (old >= 0) {
        const sib = kids(old), i = sib.indexOf(c);
        const r = orig(p, c, b);
        record("childList", old, { removed: [wrap(c)], prev: wrap(sib[i - 1]), next: wrap(sib[i + 1]) });
        const now = kids(p), j = now.indexOf(c);
        record("childList", p, { added: [wrap(c)], prev: wrap(now[j - 1]), next: wrap(now[j + 1]) });
        return r;
      }
      const r = orig(p, c, b);
      const now = kids(p), j = now.indexOf(c);
      record("childList", p, { added: [wrap(c)], prev: wrap(now[j - 1]), next: wrap(now[j + 1]) });
      return r;
    });
    hook("__detach", (orig, id) => {
      const p = __parent(id);
      if (p < 0) return orig(id);
      const sib = kids(p), i = sib.indexOf(id);
      const r = orig(id);
      record("childList", p, { removed: [wrap(id)], prev: wrap(sib[i - 1]), next: wrap(sib[i + 1]) });
      return r;
    });
    hook("__setAttr", (orig, id, name, value) => {
      const old = __getAttr(id, name);
      const r = orig(id, name, value);
      record("attributes", id, { attributeName: String(name), oldValue: old });
      return r;
    });
    hook("__removeAttr", (orig, id, name) => {
      const old = __getAttr(id, name);
      const r = orig(id, name);
      if (old != null) record("attributes", id, { attributeName: String(name), oldValue: old });
      return r;
    });
    const replaceAll = (orig, id, v) => {
      if (__type(id) === 3 || __type(id) === 8) {
        const old = __text(id);
        const r = orig(id, v);
        record("characterData", id, { oldValue: old });
        return r;
      }
      const before = kids(id).map(wrap);
      const r = orig(id, v);
      const after = kids(id).map(wrap);
      if (before.length || after.length) record("childList", id, { removed: before, added: after });
      return r;
    };
    hook("__setText", replaceAll);
    hook("__setHTML", replaceAll);
    g.MutationObserver = class MutationObserver {
      constructor(cb) {
        if (typeof cb !== "function") throw new TypeError("MutationObserver needs a callback");
        this.__cb = cb;
        this.__records = [];
      }
      observe(target, opts = {}) {
        const id = idOf(target);
        if (id < 0) throw new TypeError("observe() needs a Node");
        const o = Object.assign({}, opts);
        if (o.attributeOldValue || o.attributeFilter) o.attributes = o.attributes !== false;
        if (o.characterDataOldValue) o.characterData = o.characterData !== false;
        if (!o.childList && !o.attributes && !o.characterData) throw new TypeError("observe() needs childList, attributes or characterData");
        const existing = regs.find((r) => r.obs === this && r.target === id);
        if (existing) existing.opts = o; else regs.push({ obs: this, target: id, opts: o });
      }
      disconnect() {
        for (let i = regs.length - 1; i >= 0; i--) if (regs[i].obs === this) regs.splice(i, 1);
        this.__records = [];
        pending.delete(this);
      }
      takeRecords() { const r = this.__records.splice(0); pending.delete(this); return r; }
    };
    g.WebKitMutationObserver = g.MutationObserver;
  }

  // Node type constants (Node.ELEMENT_NODE …), on the class and its instances.
  const NODE_TYPES = { ELEMENT_NODE: 1, ATTRIBUTE_NODE: 2, TEXT_NODE: 3, CDATA_SECTION_NODE: 4, ENTITY_REFERENCE_NODE: 5, ENTITY_NODE: 6, PROCESSING_INSTRUCTION_NODE: 7, COMMENT_NODE: 8, DOCUMENT_NODE: 9, DOCUMENT_TYPE_NODE: 10, DOCUMENT_FRAGMENT_NODE: 11, NOTATION_NODE: 12, DOCUMENT_POSITION_DISCONNECTED: 1, DOCUMENT_POSITION_PRECEDING: 2, DOCUMENT_POSITION_FOLLOWING: 4, DOCUMENT_POSITION_CONTAINS: 8, DOCUMENT_POSITION_CONTAINED_BY: 16, DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC: 32 };
  for (const [k, v] of Object.entries(NODE_TYPES)) {
    if (!(k in Node)) Object.defineProperty(Node, k, { value: v });
    if (!(k in Node.prototype)) Object.defineProperty(Node.prototype, k, { value: v });
  }
  if (!document.importNode) document.importNode = (node, deep) => node.cloneNode(!!deep);
  if (!document.adoptNode) document.adoptNode = (node) => { if (node.parentNode) node.parentNode.removeChild(node); return node; };
  // window.<id>: elements with an id are reachable as globals (until a script defines the
  // name itself).
  g.__namedIds = function () {
    for (const name of __idList()) {
      if (Object.prototype.hasOwnProperty.call(g, name) || !/^[A-Za-z_$][\w$]*$/.test(name)) continue;
      Object.defineProperty(g, name, {
        configurable: true,
        enumerable: false,
        get() { return document.getElementById(name) ?? undefined; },
        set(v) { Object.defineProperty(g, name, { value: v, writable: true, configurable: true, enumerable: true }); },
      });
    }
  };
  g.HTMLSlotElement = class HTMLSlotElement extends HTMLElement { static [Symbol.hasInstance](x) { return isHtml(x) && x.localName === "slot"; } };
  g.SVGElement = class SVGElement extends Element { static [Symbol.hasInstance](x) { return isSvg(x); } };
  g.SVGGraphicsElement = class SVGGraphicsElement extends g.SVGElement { static [Symbol.hasInstance](x) { return isSvg(x); } };
  g.SVGSVGElement = class SVGSVGElement extends g.SVGElement { static [Symbol.hasInstance](x) { return isSvg(x) && x.localName === "svg"; } };
  g.HTMLUnknownElement = class HTMLUnknownElement extends HTMLElement {
    static [Symbol.hasInstance](x) { return isHtml(x) && !KNOWN_HTML.has(x.localName) && !/^[a-z][^A-Z]*-/.test(x.localName); }
  };
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
    navigator: { userAgent: __userAgent, language: __lang, languages: __lang.indexOf("-") > 0 ? [__lang, __lang.split("-")[0]] : [__lang], platform: "Linux x86_64",
                 onLine: true, cookieEnabled: true, globalPrivacyControl: true, doNotTrack: "1", hardwareConcurrency: 2, maxTouchPoints: 0, vendor: "" },
    screen: { width: 1366, height: 768, availWidth: 1366, availHeight: 768, colorDepth: 24 },
    devicePixelRatio: 1, scrollX: 0, pageXOffset: 0,
    open(url) { const u = url == null || url === "" ? "about:blank" : __resolve(location.href, String(url)); return __open(u) ? { closed: false, focus() {}, close() {}, location: { href: u }, document: null, postMessage() {} } : null; },
    scrollTo(a, b) { __scrollTo(typeof a === "object" && a ? Number(a.top || 0) : Number(b || 0)); },
    scrollBy(a, b) { __scrollTo(g.scrollY + (typeof a === "object" && a ? Number(a.top || 0) : Number(b || 0))); },
    scroll(a, b) { g.scrollTo(a, b); },
    focus() {}, blur() {}, print() {}, close() {},
    alert: (m) => __log("alert", String(m)), confirm: () => false, prompt: () => null,
    getComputedStyle: (el, pseudo) => {
      // The engine's computed values, readable as cs.fontSize, cs["font-size"] and
      // cs.getPropertyValue("font-size"). Anything it doesn't compute falls back to
      // the inline style, then "".
      let json = "";
      if (!pseudo && el && el.__id != null) json = __computed(el.__id);
      const vals = json ? JSON.parse(json) : {};
      const inline = (el && el.style) || {};
      const camel = (k) => k.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
      const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
      const get = (k) => {
        k = String(k);
        const kk = k.includes("-") || k.startsWith("--") ? k : kebab(k);
        if (kk in vals) return vals[kk];
        if (kk === "margin" || kk === "padding") return ["top", "right", "bottom", "left"].map((s) => vals[kk + "-" + s]).join(" ");
        if (kk === "border-width") return vals["border-top-width"] || "0px";
        if (kk === "border-color") return vals["border-top-color"] || "";
        if (kk === "border-style") return vals["border-top-style"] || "none";
        if (kk === "background") return vals["background-color"] || "";
        if (kk === "font") return (vals["font-style"] || "") + " " + (vals["font-weight"] || "") + " " + (vals["font-size"] || "") + " " + (vals["font-family"] || "");
        const v = inline[camel(kk)];
        return v == null || typeof v === "function" ? "" : String(v);
      };
      const cs = { getPropertyValue: get, getPropertyPriority: () => "", item: (i) => Object.keys(vals)[i] || "", get length() { return Object.keys(vals).length; }, cssText: "" };
      for (const k of Object.keys(vals)) { cs[k] = vals[k]; cs[camel(k)] = vals[k]; }
      return new Proxy(cs, { get(t, k) { if (k in t || typeof k !== "string") return t[k]; return get(k); } });
    },
    matchMedia: (q) => {
      // The same answer stylesheets get (css.rs): width, height, resolution, preferences…
      const matches = __mediaMatches(String(q));
      return { matches, media: q, onchange: null, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {}, dispatchEvent() { return true; } };
    },
    localStorage: storage(true), sessionStorage: storage(false),
    history,
    performance: { now: () => __nowMs, mark() {}, measure() {}, getEntriesByType: () => [], timing: {} },
    console: { log: consoleFn("log"), info: consoleFn("info"), debug: consoleFn("debug"),
               warn: consoleFn("warn"), error: consoleFn("error"), trace: consoleFn("trace"),
               dir: consoleFn("log"), table: consoleFn("log"), group() {}, groupEnd() {}, time() {}, timeEnd() {} },
    IntersectionObserver: class { observe() {} unobserve() {} disconnect() {} },
    ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    CSS: { supports: () => false, escape: (s) => String(s).replace(/([^\w-])/g, "\\$1") },
  });
  // Live window geometry from the layout.
  const vp = () => __viewport().split(",").map(Number);
  for (const [name, i] of [["innerWidth", 0], ["outerWidth", 0], ["innerHeight", 1], ["scrollY", 2], ["pageYOffset", 2], ["devicePixelRatio", 4]]) {
    Object.defineProperty(g, name, { get: () => (i >= 2 ? vp()[i] || (i === 4 ? 1 : 0) : Math.round(vp()[i])), configurable: true });
  }
  Object.defineProperty(g, "outerHeight", { get: () => Math.round(vp()[1]) + 80, configurable: true });

  // ------------------------------------------------------------ permissions
  // Location, notifications, camera and microphone ask first: the browser shows a prompt and
  // remembers the answer per site. (Only secure top-level pages can ask at all.)
  const permWaiters = new Map();
  let permSeq = 0;
  function permAsk(kind) {
    const st = __permState(kind);
    if (st !== "prompt") return Promise.resolve(st === "granted");
    return new Promise((res) => {
      const id = ++permSeq;
      permWaiters.set(id, res);
      if (!__permRequest(id, kind)) { permWaiters.delete(id); res(false); }
    });
  }
  g.__permAskPublic = (kind) => permAsk(kind);
  g.__permDone = function (id, granted) {
    const r = permWaiters.get(id);
    permWaiters.delete(id);
    if (r) r(!!granted);
  };
  const stateOf = (kind) => { const s = __permState(kind); return s; };
  class PermissionStatus extends EventTarget {
    constructor(name) { super(); this.name = name; this.onchange = null; }
    get state() { return stateOf(this.name); }
  }
  navigator.permissions = {
    query(desc) {
      const name = desc && desc.name;
      if (!["geolocation", "notifications", "camera", "microphone"].includes(name)) {
        return Promise.reject(new TypeError("The permission '" + name + "' isn't supported"));
      }
      return Promise.resolve(new PermissionStatus(name));
    },
  };
  // Location: asked for first; once the site is allowed, positions come from GeoClue (the
  // desktop's location service, see src/geo.rs) through __geoStart → __geoDone / __geoFail.
  class GeolocationPositionError {
    constructor(code, message) { this.code = code; this.message = message; }
    get [Symbol.toStringTag]() { return "GeolocationPositionError"; }
  }
  for (const [k, v] of [["PERMISSION_DENIED", 1], ["POSITION_UNAVAILABLE", 2], ["TIMEOUT", 3]]) {
    Object.defineProperty(GeolocationPositionError, k, { value: v, enumerable: true });
    Object.defineProperty(GeolocationPositionError.prototype, k, { value: v, enumerable: true });
  }
  class GeolocationCoordinates {
    constructor(c) {
      for (const k of ["accuracy", "latitude", "longitude", "altitude", "altitudeAccuracy", "heading", "speed"]) {
        Object.defineProperty(this, k, { value: c[k] === undefined ? null : c[k], enumerable: true });
      }
    }
    toJSON() {
      const o = {};
      for (const k of ["accuracy", "latitude", "longitude", "altitude", "altitudeAccuracy", "heading", "speed"]) o[k] = this[k];
      return o;
    }
  }
  class GeolocationPosition {
    constructor(p) {
      Object.defineProperty(this, "coords", { value: new GeolocationCoordinates(p), enumerable: true });
      Object.defineProperty(this, "timestamp", { value: p.timestamp, enumerable: true });
    }
    toJSON() { return { coords: this.coords.toJSON(), timestamp: this.timestamp }; }
  }
  // Requests waiting for an answer: id → { ok, err, once, timer }.
  const geoReqs = new Map();
  let geoSeq = 0;
  const geoFinish = (id) => {
    const r = geoReqs.get(id);
    if (!r) return null;
    if (r.timer) clearTimeout(r.timer);
    r.timer = null;
    if (r.once) geoReqs.delete(id);
    return r;
  };
  const geoCall = (f, arg) => { try { f(arg); } catch (e) { console.error(String(e)); if (g.__onListenerError) g.__onListenerError(e); } };
  g.__geoDone = function (id, p) {
    const r = geoFinish(id);
    if (!r) return;
    geoCall(r.ok, new GeolocationPosition(p));
    // A watch waits again for the next position, with the same timeout.
    if (!r.once && geoReqs.has(id)) geoArm(id, r, r.timeout);
  };
  g.__geoFail = function (id, code, msg) {
    const r = geoReqs.get(id);
    if (!r) return;
    geoReqs.delete(id);
    if (r.timer) clearTimeout(r.timer);
    if (typeof r.err === "function") geoCall(r.err, new GeolocationPositionError(code, msg));
  };
  const geoOpts = (o) => {
    o = o == null ? {} : o;
    const ms = (v, d) => { v = v === undefined ? d : Math.floor(Number(v)); return Number.isFinite(v) ? Math.min(Math.max(v, 0), 0xFFFFFFFF) : (v > 0 ? 0xFFFFFFFF : 0); };
    return { high: !!o.enableHighAccuracy, timeout: ms(o.timeout, 0xFFFFFFFF), maxAge: ms(o.maximumAge, 0) };
  };
  // Arm the timeout for one wait for a position (it counts from when the site is allowed).
  const geoArm = (id, r, timeout) => {
    if (timeout >= 0xFFFFFFFF) return;
    r.timer = setTimeout(() => {
      r.timer = null;
      if (!geoReqs.has(id)) return;
      if (r.once) { geoReqs.delete(id); __geoStop(id); }
      if (typeof r.err === "function") geoCall(r.err, new GeolocationPositionError(3, "Timeout expired"));
    }, timeout);
  };
  const geoRequest = (ok, err, options, once) => {
    if (typeof ok !== "function") throw new TypeError("Geolocation: the success callback isn't a function");
    const o = geoOpts(options);
    const id = ++geoSeq;
    const r = { ok, err, once, timer: null, timeout: o.timeout };
    geoReqs.set(id, r);
    permAsk("geolocation").then((granted) => {
      if (!geoReqs.has(id)) return; // cleared while asking
      if (!granted) { g.__geoFail(id, 1, "User denied Geolocation"); return; }
      if (!__geoStart(id, once, o.high, o.maxAge)) return; // fails through __geoFail
      // (An answer from the last position is delivered before any timer runs.)
      geoArm(id, r, o.timeout);
    });
    return id;
  };
  class Geolocation {
    getCurrentPosition(ok, err, options) { geoRequest(ok, err, options, true); }
    watchPosition(ok, err, options) { return geoRequest(ok, err, options, false); }
    clearWatch(id) {
      const r = geoReqs.get(id);
      if (!r || r.once) return;
      if (r.timer) clearTimeout(r.timer);
      geoReqs.delete(id);
      __geoStop(id);
    }
  }
  Object.assign(g, { Geolocation, GeolocationPosition, GeolocationCoordinates, GeolocationPositionError });
  navigator.geolocation = new Geolocation();
  // Notifications: shown on the desktop once the site is allowed.
  class Notification extends EventTarget {
    constructor(title, opts) {
      super();
      opts = opts || {};
      this.title = String(title); this.body = String(opts.body || ""); this.tag = opts.tag || ""; this.data = opts.data;
      this.onclick = this.onshow = this.onclose = this.onerror = null;
      const shown = __notify(this.title, this.body);
      setTimeout(() => this.__fire(new Event(shown ? "show" : "error")), 0);
    }
    static get permission() { const s = stateOf("notifications"); return s === "prompt" ? "default" : s; }
    static requestPermission(cb) {
      return permAsk("notifications").then((granted) => {
        const r = granted ? "granted" : "denied";
        if (typeof cb === "function") cb(r);
        return r;
      });
    }
    close() {}
  }
  g.Notification = Notification;
  // Camera and microphone: asked for; Ironswift can't capture video or sound yet, so an
  // allowed request fails with NotReadableError.
  navigator.mediaDevices = {
    getUserMedia(c) {
      c = c || {};
      const kinds = [];
      if (c.video) kinds.push("camera");
      if (c.audio) kinds.push("microphone");
      if (!kinds.length) return Promise.reject(new TypeError("getUserMedia needs audio or video"));
      return kinds.reduce((p, k) => p.then((all) => permAsk(k).then((ok) => all && ok)), Promise.resolve(true)).then((ok) => {
        if (!ok) throw new DOMException("Permission denied", "NotAllowedError");
        throw new DOMException("Ironswift can't use cameras or microphones yet", "NotReadableError");
      });
    },
    enumerateDevices() { return Promise.resolve([]); },
    addEventListener() {}, removeEventListener() {},
  };
})();
