// IntersectionObserver and ResizeObserver. Both are checked after each layout (the browser
// calls __afterLayout, which runs __layoutHooks) against getBoundingClientRect, and only when
// something could have changed: the layout, the scroll position, the window size, or the
// list of observed elements.
(function () {
  "use strict";
  const g = globalThis;
  const hooks = (g.__layoutHooks ||= []);

  class DOMRectReadOnly {
    constructor(x = 0, y = 0, width = 0, height = 0) {
      Object.defineProperties(this, {
        x: { value: +x, enumerable: true }, y: { value: +y, enumerable: true },
        width: { value: +width, enumerable: true }, height: { value: +height, enumerable: true },
      });
    }
    get top() { return Math.min(this.y, this.y + this.height); }
    get left() { return Math.min(this.x, this.x + this.width); }
    get bottom() { return Math.max(this.y, this.y + this.height); }
    get right() { return Math.max(this.x, this.x + this.width); }
    toJSON() { return { x: this.x, y: this.y, width: this.width, height: this.height, top: this.top, right: this.right, bottom: this.bottom, left: this.left }; }
    static fromRect(r = {}) { return new this(r.x, r.y, r.width, r.height); }
  }
  class DOMRect extends DOMRectReadOnly {
    constructor(x = 0, y = 0, width = 0, height = 0) {
      super();
      const v = { x: +x, y: +y, width: +width, height: +height };
      for (const k of Object.keys(v)) Object.defineProperty(this, k, { get: () => v[k], set: (n) => { v[k] = +n; }, enumerable: true });
    }
  }
  if (!g.DOMRectReadOnly) g.DOMRectReadOnly = DOMRectReadOnly;
  if (!g.DOMRect) g.DOMRect = DOMRect;
  const rectOf = (el) => {
    if (!el.isConnected) return new DOMRectReadOnly();
    const r = el.getBoundingClientRect();
    return new DOMRectReadOnly(r.left, r.top, r.width, r.height);
  };

  // Something that could move boxes since the last check?
  let stamp = "";
  let force = true;
  const changed = () => {
    const s = __geomGen() + "|" + g.scrollY + "|" + g.innerWidth + "x" + g.innerHeight + "|" + __elScrollGen();
    if (s === stamp && !force) return false;
    stamp = s;
    force = false;
    return true;
  };

  // ------------------------------------------------------------ IntersectionObserver
  const ios = new Set();
  function parseMargin(m) {
    const parts = String(m == null ? "0px" : m).trim().split(/\s+/);
    if (parts.length > 4 || parts.some((p) => !/^-?(\d+(\.\d+)?|\.\d+)(px|%)$|^0$/.test(p))) {
      throw new DOMException("rootMargin must be given in pixels or percent", "SyntaxError");
    }
    const v = parts.map((p) => (p === "0" ? { n: 0, pct: false } : { n: parseFloat(p), pct: p.endsWith("%") }));
    const [t, r = t, b = t, l = r] = v;
    return [t, r, b, l];
  }
  const marginText = (m) => m.map((x) => x.n + (x.pct ? "%" : "px")).join(" ");
  class IntersectionObserverEntry {
    constructor(init) { Object.assign(this, init); }
  }
  class IntersectionObserver {
    constructor(callback, options = {}) {
      if (typeof callback !== "function") throw new TypeError("IntersectionObserver needs a callback");
      let t = options.threshold === undefined ? [0] : Array.isArray(options.threshold) ? options.threshold.slice() : [options.threshold];
      t = t.map(Number);
      if (t.some((x) => !(x >= 0 && x <= 1))) throw new RangeError("threshold values must be between 0 and 1");
      if (!t.length) t = [0];
      t.sort((a, b) => a - b);
      const root = options.root ?? null;
      if (root !== null && !(root.nodeType === 1 || root.nodeType === 9)) throw new TypeError("root must be an Element or Document");
      this.__cb = callback;
      this.__margin = parseMargin(options.rootMargin);
      this.__targets = new Map(); // element -> { index, intersecting }
      this.__records = [];
      Object.defineProperties(this, {
        root: { value: root, enumerable: true },
        rootMargin: { value: marginText(this.__margin), enumerable: true },
        thresholds: { value: Object.freeze(t), enumerable: true },
        scrollMargin: { value: "0px 0px 0px 0px", enumerable: true },
        delay: { value: 0, enumerable: true },
        trackVisibility: { value: false, enumerable: true },
      });
    }
    observe(target) {
      if (!target || target.nodeType !== 1) throw new TypeError("observe() needs an Element");
      if (this.__targets.has(target)) return;
      this.__targets.set(target, { index: -1, intersecting: false });
      ios.add(this);
      force = true;
    }
    unobserve(target) { this.__targets.delete(target); if (!this.__targets.size) ios.delete(this); }
    disconnect() { this.__targets.clear(); ios.delete(this); }
    takeRecords() { return this.__records.splice(0); }
    __rootBounds() {
      let r;
      if (this.root && this.root.nodeType === 1) r = rectOf(this.root);
      else r = new DOMRectReadOnly(0, 0, g.innerWidth, g.innerHeight);
      const [t, ri, b, l] = this.__margin;
      const px = (m, size) => (m.pct ? (m.n * size) / 100 : m.n);
      return new DOMRectReadOnly(r.x - px(l, r.width), r.y - px(t, r.height), r.width + px(l, r.width) + px(ri, r.width), r.height + px(t, r.height) + px(b, r.height));
    }
    __check(now) {
      const rootBounds = this.__rootBounds();
      for (const [el, st] of this.__targets) {
        const box = rectOf(el);
        let inRoot = el.isConnected;
        if (inRoot && this.root && this.root.nodeType === 1) inRoot = this.root !== el && this.root.contains(el);
        let ix = new DOMRectReadOnly();
        let intersecting = false;
        if (inRoot) {
          const x0 = Math.max(box.left, rootBounds.left), y0 = Math.max(box.top, rootBounds.top);
          const x1 = Math.min(box.right, rootBounds.right), y1 = Math.min(box.bottom, rootBounds.bottom);
          // Edge-adjacent boxes (and zero-size targets inside the root) count as intersecting.
          if (x1 >= x0 && y1 >= y0) { intersecting = true; ix = new DOMRectReadOnly(x0, y0, x1 - x0, y1 - y0); }
        }
        const area = box.width * box.height;
        const ratio = intersecting ? (area > 0 ? (ix.width * ix.height) / area : 1) : 0;
        let index = 0;
        if (intersecting) { index = this.thresholds.findIndex((x) => x > ratio); if (index < 0) index = this.thresholds.length; }
        if (index === st.index && intersecting === st.intersecting) continue;
        st.index = index;
        st.intersecting = intersecting;
        this.__records.push(new IntersectionObserverEntry({
          time: now, rootBounds: this.root && this.root.nodeType === 1 ? rootBounds : rootBounds, boundingClientRect: box,
          intersectionRect: ix, isIntersecting: intersecting, intersectionRatio: ratio, target: el, isVisible: false,
        }));
      }
      if (this.__records.length) {
        // The callback runs in its own task, like browsers queue it.
        __queueTask(() => {
          const recs = this.__records.splice(0);
          if (recs.length) this.__cb.call(this, recs, this);
        });
      }
    }
  }
  g.IntersectionObserver = IntersectionObserver;
  g.IntersectionObserverEntry = IntersectionObserverEntry;

  // ------------------------------------------------------------ ResizeObserver
  const ros = new Set();
  const px = (v) => parseFloat(v) || 0;
  function sizes(el) {
    const r = el.getBoundingClientRect();
    const bw = r.width, bh = r.height;
    let cs = null;
    try { cs = getComputedStyle(el); } catch (e) { cs = null; }
    const padL = cs ? px(cs.paddingLeft) : 0, padT = cs ? px(cs.paddingTop) : 0;
    const padX = cs ? padL + px(cs.paddingRight) : 0, padY = cs ? padT + px(cs.paddingBottom) : 0;
    const brX = cs ? px(cs.borderLeftWidth) + px(cs.borderRightWidth) : 0, brY = cs ? px(cs.borderTopWidth) + px(cs.borderBottomWidth) : 0;
    const shown = el.isConnected && (bw > 0 || bh > 0 || (cs && cs.display !== "none"));
    if (!shown) return { border: [0, 0], content: [0, 0], pad: [0, 0] };
    return { border: [bw, bh], content: [Math.max(0, bw - padX - brX), Math.max(0, bh - padY - brY)], pad: [padL, padT] };
  }
  class ResizeObserverEntry {
    constructor(el, s) {
      const box = (w, h) => Object.freeze([Object.freeze({ inlineSize: w, blockSize: h })]);
      Object.defineProperties(this, {
        target: { value: el, enumerable: true },
        contentRect: { value: new DOMRectReadOnly(s.pad[0], s.pad[1], s.content[0], s.content[1]), enumerable: true },
        borderBoxSize: { value: box(s.border[0], s.border[1]), enumerable: true },
        contentBoxSize: { value: box(s.content[0], s.content[1]), enumerable: true },
        devicePixelContentBoxSize: { value: box(s.content[0] * (g.devicePixelRatio || 1), s.content[1] * (g.devicePixelRatio || 1)), enumerable: true },
      });
    }
  }
  class ResizeObserverSize {}
  class ResizeObserver {
    constructor(callback) {
      if (typeof callback !== "function") throw new TypeError("ResizeObserver needs a callback");
      this.__cb = callback;
      this.__targets = new Map(); // element -> { box, last: [w, h] }
    }
    observe(target, options = {}) {
      if (!target || target.nodeType !== 1) throw new TypeError("observe() needs an Element");
      const box = options.box || "content-box";
      if (!["content-box", "border-box", "device-pixel-content-box"].includes(box)) throw new TypeError("unknown box: " + box);
      // A new observation always reports once (the last size starts out impossible).
      this.__targets.set(target, { box, last: [-1, -1] });
      ros.add(this);
      force = true;
    }
    unobserve(target) { this.__targets.delete(target); if (!this.__targets.size) ros.delete(this); }
    disconnect() { this.__targets.clear(); ros.delete(this); }
    __check() {
      const entries = [];
      for (const [el, st] of this.__targets) {
        const s = sizes(el);
        const now = st.box === "border-box" ? s.border : s.content;
        if (now[0] === st.last[0] && now[1] === st.last[1]) continue;
        st.last = now;
        entries.push(new ResizeObserverEntry(el, s));
      }
      if (entries.length) {
        try { this.__cb.call(this, entries, this); } catch (e) { console.error(String(e)); if (g.__onListenerError) g.__onListenerError(e); }
      }
    }
  }
  g.ResizeObserver = ResizeObserver;
  g.ResizeObserverEntry = ResizeObserverEntry;
  g.ResizeObserverSize = ResizeObserverSize;

  hooks.push(() => {
    if (!ios.size && !ros.size) return;
    if (!changed()) return;
    const now = typeof performance !== "undefined" ? performance.now() : __nowMs;
    for (const o of [...ios]) o.__check(now);
    // Resize callbacks may change sizes again: check a few rounds, like browsers' depth loop.
    for (let round = 0; round < 4 && ros.size; round++) {
      for (const o of [...ros]) o.__check();
      if (!changed()) break;
    }
  });
})();
