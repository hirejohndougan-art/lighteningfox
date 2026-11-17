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

  function wrap(id) {
    if (id === undefined || id === null || id < 0) return null;
    let n = cache[id];
    if (n) return n;
    const t = __type(id);
    n = t === 9 ? document : t === 3 ? new Text(id) : t === 8 ? new Comment(id) : t === 11 && __shadowHost(id) ? new ShadowRoot(id, SHADOW_KEY) : t === 11 ? new DocumentFragment(id) : new Element(id);
    cache[id] = n;
    return n;
  }
  globalThis.__wrapFn = wrap;
  const idOf = (n) => (n && n.__id !== undefined ? n.__id : -1);
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
        } catch (e) { console.error(String(e)); }
        if (ev._stopNow) return;
      }
      if (phase === "capture") return;
      const prop = this["on" + ev.type];
      if (typeof prop === "function") {
        try { if (prop.call(this, ev) === false) ev.preventDefault(); } catch (e) { console.error(String(e)); }
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
