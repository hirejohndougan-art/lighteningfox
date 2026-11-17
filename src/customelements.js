// Custom elements (web components): customElements.define and the element lifecycle.
//
// Element objects are made by the prelude's wrap(id). Upgrading one sets its prototype to
// the page's class and runs the class constructor, whose super() call (HTMLElement below)
// hands back that same object, so `this` in the constructor is the existing element.
// The tree-changing natives are wrapped so connected/disconnected/attributeChanged
// callbacks run right after the change, as the spec's "custom element reactions" do.
(function () {
  "use strict";
  const g = globalThis;
  const wrap = g.__wrapFn;
  const cache = g.__wrapCache;
  const Base = g.HTMLElement;
  const defs = new Map(); // name -> definition
  const byCtor = new Map(); // constructor -> definition
  const waiting = new Map(); // name -> [resolve]
  const upgraded = new WeakSet();
  const stack = []; // elements being upgraded (the constructor's super() returns the top)
  const report = (e) => { console.error(String(e && e.stack ? e : e)); if (g.__onListenerError) g.__onListenerError(e); };

  const RESERVED = new Set(["annotation-xml", "color-profile", "font-face", "font-face-src", "font-face-uri",
    "font-face-format", "font-face-name", "missing-glyph"]);
  const validName = (n) => /^[a-z][-.0-9_a-z·À-￿]*-[-.0-9_a-z·À-￿]*$/.test(n) && !RESERVED.has(n);

  // The HTMLElement constructor pages extend.
  function HTMLElement() {
    const def = new.target && byCtor.get(new.target);
    if (!def) throw new TypeError("Illegal constructor");
    let el = stack.length ? stack[stack.length - 1] : null;
    if (el && el.__ceConstructed) throw new DOMException("This element was already constructed", "InvalidStateError");
    if (!el) { el = wrap(__create(def.name)); el.__ceDef = def; } // `new MyElement()` called directly
    else el.__ceConstructed = true;
    Object.setPrototypeOf(el, new.target.prototype);
    upgraded.add(el);
    return el;
  }
  HTMLElement.prototype = Base.prototype;
  Object.defineProperty(Base.prototype, "constructor", { value: HTMLElement, writable: true, configurable: true });
  Object.setPrototypeOf(HTMLElement, Object.getPrototypeOf(Base));
  Base.prototype.attachInternals = function () {
    if (this.__internals) throw new DOMException("attachInternals() was already called", "NotSupportedError");
    const el = this;
    this.__internals = {
      get form() { return el.closest ? el.closest("form") : null; },
      states: new Set(), validity: { valid: true }, validationMessage: "", willValidate: false,
      get labels() { return []; }, shadowRoot: null,
      setFormValue(v) { el.__formValue = v; }, setValidity() {}, checkValidity: () => true, reportValidity: () => true,
    };
    return this.__internals;
  };
  // `x instanceof HTMLElement`: any element in the HTML namespace (node objects all share
  // Element.prototype). Page subclasses keep the ordinary prototype check.
  Object.defineProperty(HTMLElement, Symbol.hasInstance, {
    value(x) { return this === HTMLElement ? g.__isHtmlElement(x) || Function.prototype[Symbol.hasInstance].call(this, x) : Function.prototype[Symbol.hasInstance].call(this, x); },
    configurable: true,
  });
  g.HTMLElement = HTMLElement;

  // Namespaced attribute methods (attributes here have no namespaces; HTML pages rarely
  // need them, but libraries and custom-element code call them).
  const EP = Object.getPrototypeOf(Base.prototype);
  const local = (n) => { n = String(n); const i = n.indexOf(":"); return i < 0 ? n : n.slice(i + 1); };
  if (!EP.getAttributeNS) EP.getAttributeNS = function (_ns, n) { return this.getAttribute(local(n)); };
  if (!EP.hasAttributeNS) EP.hasAttributeNS = function (_ns, n) { return this.hasAttribute(local(n)); };
  if (!EP.setAttributeNS) EP.setAttributeNS = function (_ns, n, v) { this.setAttribute(local(n), v); };
  if (!EP.removeAttributeNS) EP.removeAttributeNS = function (_ns, n) { this.removeAttribute(local(n)); };

  function call(el, name, args) {
    const def = el.__ceDef;
    const fn = def && def.callbacks[name];
    if (typeof fn !== "function") return;
    try { fn.apply(el, args); } catch (e) { report(e); }
  }

  function upgrade(el, def) {
    if (upgraded.has(el) || el.__ceFailed) return;
    el.__ceDef = def;
    stack.push(el);
    try {
      const r = new def.ctor();
      if (r !== el) throw new TypeError("custom element constructor must not return a different object");
    } catch (e) {
      el.__ceFailed = true;
      delete el.__ceDef;
      Object.setPrototypeOf(el, Base.prototype);
      report(e);
      return;
    } finally {
      stack.pop();
      delete el.__ceConstructed;
    }
    for (const a of def.observed) {
      const v = el.getAttribute(a);
      if (v !== null) call(el, "attributeChangedCallback", [a, null, v, null]);
    }
    if (connected(el.__id)) call(el, "connectedCallback", []);
  }

  // Is the node in the document (through shadow roots to their hosts)?
  function connected(id) {
    let n = id, guard = 0;
    while (n > 0 && guard++ < 100000) {
      const p = __parent(n);
      if (p < 0) {
        const host = __type(n) === 11 ? __shadowHost(n) : null;
        if (!host) return false;
        n = host[0];
      } else n = p;
    }
    return n === 0;
  }

  // Every element at or under `id` (shadow trees included), in tree order.
  function eachElement(id, fn) {
    const todo = [id];
    while (todo.length) {
      const n = todo.pop();
      if (__type(n) === 1) {
        fn(n);
        const sr = __shadowRoot(n, true);
        const kids = __children(n);
        for (let i = kids.length - 1; i >= 0; i--) todo.push(kids[i]);
        if (sr >= 0) todo.push(sr);
      } else {
        const kids = __children(n) || [];
        for (let i = kids.length - 1; i >= 0; i--) todo.push(kids[i]);
      }
    }
  }

  // After nodes went into the document: upgrade defined ones, tell upgraded ones.
  // With `upgradeOnly`, elements already upgraded are left alone (a sweep, not an insertion).
  function connectedTree(id, upgradeOnly) {
    if (!defs.size || !connected(id)) return;
    eachElement(id, (n) => {
      const o = cache[n];
      if (o && upgraded.has(o)) { if (!upgradeOnly) call(o, "connectedCallback", []); return; }
      const def = defs.get(__tag(n));
      if (def) upgrade(wrap(n), def);
    });
  }
  // Before nodes leave the document: the upgraded ones that will be disconnected.
  function upgradedUnder(id) {
    const out = [];
    if (!defs.size || !connected(id)) return out;
    eachElement(id, (n) => { const o = cache[n]; if (o && upgraded.has(o)) out.push(o); });
    return out;
  }
  const disconnect = (list) => { for (const o of list) call(o, "disconnectedCallback", []); };

  const hook = (name, fn) => {
    const orig = g[name];
    g[name] = function (...args) { return defs.size ? fn(orig, ...args) : orig.apply(this, args); };
  };
  hook("__insert", (orig, p, c, b) => {
    const leaving = __parent(c) >= 0 ? upgradedUnder(c) : [];
    const r = orig(p, c, b);
    disconnect(leaving);
    connectedTree(c);
    return r;
  });
  hook("__detach", (orig, id) => {
    const leaving = upgradedUnder(id);
    const r = orig(id);
    disconnect(leaving);
    return r;
  });
  const replaceChildren = (orig, id, v) => {
    const leaving = [];
    for (const k of __children(id) || []) leaving.push(...upgradedUnder(k));
    const r = orig(id, v);
    disconnect(leaving);
    for (const k of __children(id) || []) connectedTree(k);
    return r;
  };
  hook("__setHTML", replaceChildren);
  hook("__setText", replaceChildren);
  const attrChanged = (id, name, old, value) => {
    const o = cache[id];
    if (!o || !upgraded.has(o) || !o.__ceDef.observed.includes(name)) return;
    call(o, "attributeChangedCallback", [name, old, value, null]);
  };
  hook("__setAttr", (orig, id, name, value) => {
    const old = __getAttr(id, name);
    const r = orig(id, name, value);
    attrChanged(id, String(name), old, String(value));
    return r;
  });
  hook("__removeAttr", (orig, id, name) => {
    const old = __getAttr(id, name);
    const r = orig(id, name);
    if (old !== null) attrChanged(id, String(name), old, null);
    return r;
  });

  // createElement upgrades right away, even before the element is in the document.
  const createElement = document.createElement;
  document.createElement = function (t, opts) {
    const el = createElement.call(this, t, opts);
    const def = defs.size && defs.get(el.localName);
    if (def) upgrade(el, def);
    return el;
  };

  // Elements the parser made after define(): upgrade before each script and at the end.
  const upgradeDocument = () => { if (defs.size) connectedTree(0, true); };
  for (const name of ["__namedIds", "__loaded"]) {
    const orig = g[name];
    g[name] = function (...a) { upgradeDocument(); return orig.apply(this, a); };
  }

  class CustomElementRegistry {
    define(name, ctor, options) {
      name = String(name);
      if (typeof ctor !== "function") throw new TypeError("customElements.define needs a constructor");
      if (!validName(name)) throw new DOMException(`"${name}" is not a valid custom element name`, "SyntaxError");
      if (defs.has(name)) throw new DOMException(`"${name}" has already been defined`, "NotSupportedError");
      if (byCtor.has(ctor)) throw new DOMException("this constructor has already been defined", "NotSupportedError");
      if (options && options.extends) return; // customized built-ins (is="…"): not supported, like Safari
      const proto = ctor.prototype;
      if (proto === null || typeof proto !== "object") throw new TypeError("constructor prototype must be an object");
      const callbacks = {};
      for (const k of ["connectedCallback", "disconnectedCallback", "adoptedCallback", "attributeChangedCallback"]) callbacks[k] = proto[k];
      let observed = [];
      if (typeof callbacks.attributeChangedCallback === "function" && ctor.observedAttributes != null) observed = Array.from(ctor.observedAttributes, String);
      const def = { name, ctor, callbacks, observed };
      defs.set(name, def);
      byCtor.set(ctor, def);
      connectedTree(0, true);
      for (const r of waiting.get(name) || []) r(ctor);
      waiting.delete(name);
    }
    get(name) { const d = defs.get(String(name)); return d ? d.ctor : undefined; }
    getName(ctor) { const d = byCtor.get(ctor); return d ? d.name : null; }
    whenDefined(name) {
      name = String(name);
      if (!validName(name)) return Promise.reject(new DOMException(`"${name}" is not a valid custom element name`, "SyntaxError"));
      const d = defs.get(name);
      if (d) return Promise.resolve(d.ctor);
      return new Promise((res) => { if (!waiting.has(name)) waiting.set(name, []); waiting.get(name).push(res); });
    }
    upgrade(root) {
      const id = root && root.__id;
      if (id === undefined) return;
      eachElement(id, (n) => { const def = defs.get(__tag(n)); if (def) upgrade(wrap(n), def); });
    }
  }
  g.CustomElementRegistry = CustomElementRegistry;
  g.customElements = new CustomElementRegistry();
})();
