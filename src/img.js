// <img> loading for scripts: an image whose src a script sets (new Image(), createElement,
// in the document or not) loads on its own and fires load or error; complete,
// naturalWidth/naturalHeight, width/height and decode() reflect it. img_load.rs fetches.
(function () {
  const g = globalThis;
  if (g.__imgInstalled) return;
  g.__imgInstalled = true;

  const state = new Map(); // node id -> { status: "loading" | "ok" | "error", token, w, h, waiters }
  const byToken = new Map(); // token -> element
  const queue = [];
  let nextToken = 1;
  let polling = false;

  const isImg = (el) => el && el.nodeType === 1 && el.localName === "img";
  const hasSource = (el) => !!(el.getAttribute("src") || el.getAttribute("srcset"));

  function start(el) {
    const id = el.__id;
    const old = state.get(id);
    if (old && old.token) byToken.delete(old.token);
    if (!hasSource(el)) {
      state.set(id, { status: "empty", w: 0, h: 0, waiters: old ? old.waiters : [] });
      settle(el, state.get(id), false, true);
      return;
    }
    const token = nextToken++;
    state.set(id, { status: "loading", token, w: 0, h: 0, waiters: old ? old.waiters : [] });
    byToken.set(token, el);
    queue.push([token, el]);
    pump();
  }

  function pump() {
    while (queue.length) {
      const [token, el] = queue[0];
      if (!byToken.has(token)) { queue.shift(); continue; }
      if (!__imgStart(token, el.getAttribute("src") || "", el.__id)) break;
      queue.shift();
    }
    if (!polling) {
      polling = true;
      setTimeout(poll, 4);
    }
  }

  function settle(el, s, ok, quiet) {
    const waiters = s.waiters.splice(0);
    for (const [res, rej] of waiters) (ok ? res : rej)(ok ? undefined : new DOMException("The image couldn't be decoded.", "EncodingError"));
    if (!quiet) el.dispatchEvent(new Event(ok ? "load" : "error"));
  }

  function poll() {
    polling = false;
    const rows = __imgPoll();
    let running = 0;
    for (const [token, ok, w, h] of rows) {
      if (token === -1) { running = ok; continue; }
      const el = byToken.get(token);
      byToken.delete(token);
      if (!el) continue;
      const s = state.get(el.__id);
      if (!s || s.token !== token) continue;
      s.status = ok ? "ok" : "error";
      s.w = ok ? w : 0;
      s.h = ok ? h : 0;
      settle(el, s, ok, false);
    }
    if (running > 0 || queue.length) pump();
  }

  // Parser-made images report what the page loaded; ask about them and they load here too.
  function info(el) {
    let s = state.get(el.__id);
    if (s) return s;
    if (!hasSource(el)) return { status: "empty", w: 0, h: 0 };
    const size = typeof __cvSourceSize === "function" ? __cvSourceSize(0, String(el.__id)) : null;
    if (size) return { status: "ok", w: size[0], h: size[1] };
    start(el);
    return state.get(el.__id);
  }

  const proto = g.Element.prototype;
  const setAttr = proto.setAttribute;
  proto.setAttribute = function (name, value) {
    const r = setAttr.call(this, name, value);
    const n = String(name).toLowerCase();
    if (isImg(this) && (n === "src" || n === "srcset")) start(this);
    return r;
  };
  const addListener = g.EventTarget && g.EventTarget.prototype.addEventListener;
  if (addListener) {
    g.EventTarget.prototype.addEventListener = function (type, fn, opts) {
      const r = addListener.call(this, type, fn, opts);
      if ((type === "load" || type === "error") && isImg(this) && !state.has(this.__id) && hasSource(this) && !(typeof __cvSourceSize === "function" && __cvSourceSize(0, String(this.__id)))) start(this);
      return r;
    };
  }

  function imgProp(name, get, set) {
    const prev = Object.getOwnPropertyDescriptor(proto, name) || Object.getOwnPropertyDescriptor(g.HTMLElement.prototype, name);
    Object.defineProperty(proto, name, {
      configurable: true,
      get() {
        if (isImg(this)) return get.call(this);
        if (prev && prev.get) return prev.get.call(this);
        return prev ? prev.value : undefined;
      },
      set(v) {
        if (isImg(this) && set) return set.call(this, v);
        if (prev && prev.set) return prev.set.call(this, v);
        Object.defineProperty(this, name, { value: v, writable: true, configurable: true, enumerable: true });
      },
    });
  }
  imgProp("complete", function () { const s = info(this); return s.status !== "loading"; });
  imgProp("naturalWidth", function () { return info(this).w; });
  imgProp("naturalHeight", function () { return info(this).h; });
  imgProp("currentSrc", function () { const s = this.getAttribute("src"); return s ? new URL(s, document.baseURI || location.href).href : ""; });
  for (const [name, i] of [["width", "w"], ["height", "h"]]) {
    imgProp(name, function () {
      const a = this.getAttribute(name);
      if (a != null && /^\s*\d/.test(a)) return parseInt(a, 10);
      return info(this)[i];
    }, function (v) { this.setAttribute(name, String(Math.max(0, v | 0))); });
  }
  imgProp("crossOrigin", function () { return this.getAttribute("crossorigin"); }, function (v) {
    if (v == null) this.removeAttribute("crossorigin"); else this.setAttribute("crossorigin", String(v));
  });
  imgProp("decode", function () {
    const el = this;
    return function decode() {
      const s = info(el);
      if (s.status === "ok") return Promise.resolve();
      if (s.status !== "loading") return Promise.reject(new DOMException("The image couldn't be decoded.", "EncodingError"));
      return new Promise((res, rej) => s.waiters.push([res, rej]));
    };
  });
})();
