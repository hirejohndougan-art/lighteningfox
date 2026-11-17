// Geometry interfaces: DOMPoint, DOMRect, DOMQuad and DOMMatrix (with their read-only bases).
// Canvas transforms (getTransform, setTransform, Path2D.addPath) and roundRect radii use them.
(function () {
  const g = globalThis;
  if (typeof g.DOMMatrix === "function") return;
  const num = (v, d) => (v === undefined ? d : Number(v));

  class DOMPointReadOnly {
    constructor(x = 0, y = 0, z = 0, w = 1) { this.__p = [Number(x), Number(y), Number(z), Number(w)]; }
    static fromPoint(p = {}) { return new this(num(p.x, 0), num(p.y, 0), num(p.z, 0), num(p.w, 1)); }
    get x() { return this.__p[0]; } get y() { return this.__p[1]; } get z() { return this.__p[2]; } get w() { return this.__p[3]; }
    matrixTransform(m) { return DOMMatrixReadOnly.fromMatrix(m).transformPoint(this); }
    toJSON() { return { x: this.x, y: this.y, z: this.z, w: this.w }; }
  }
  class DOMPoint extends DOMPointReadOnly {
    set x(v) { this.__p[0] = Number(v); } get x() { return this.__p[0]; }
    set y(v) { this.__p[1] = Number(v); } get y() { return this.__p[1]; }
    set z(v) { this.__p[2] = Number(v); } get z() { return this.__p[2]; }
    set w(v) { this.__p[3] = Number(v); } get w() { return this.__p[3]; }
  }

  class DOMRectReadOnly {
    constructor(x = 0, y = 0, width = 0, height = 0) { this.__r = [Number(x), Number(y), Number(width), Number(height)]; }
    static fromRect(r = {}) { return new this(num(r.x, 0), num(r.y, 0), num(r.width, 0), num(r.height, 0)); }
    get x() { return this.__r[0]; } get y() { return this.__r[1]; } get width() { return this.__r[2]; } get height() { return this.__r[3]; }
    get top() { return Math.min(this.y, this.y + this.height); } get bottom() { return Math.max(this.y, this.y + this.height); }
    get left() { return Math.min(this.x, this.x + this.width); } get right() { return Math.max(this.x, this.x + this.width); }
    toJSON() { return { x: this.x, y: this.y, width: this.width, height: this.height, top: this.top, right: this.right, bottom: this.bottom, left: this.left }; }
  }
  class DOMRect extends DOMRectReadOnly {
    set x(v) { this.__r[0] = Number(v); } get x() { return this.__r[0]; }
    set y(v) { this.__r[1] = Number(v); } get y() { return this.__r[1]; }
    set width(v) { this.__r[2] = Number(v); } get width() { return this.__r[2]; }
    set height(v) { this.__r[3] = Number(v); } get height() { return this.__r[3]; }
  }

  class DOMQuad {
    constructor(p1 = {}, p2 = {}, p3 = {}, p4 = {}) { [this.p1, this.p2, this.p3, this.p4] = [p1, p2, p3, p4].map((p) => DOMPoint.fromPoint(p)); }
    static fromRect(r = {}) { const x = num(r.x, 0), y = num(r.y, 0), w = num(r.width, 0), h = num(r.height, 0); return new DOMQuad({ x, y }, { x: x + w, y }, { x: x + w, y: y + h }, { x, y: y + h }); }
    static fromQuad(q = {}) { return new DOMQuad(q.p1, q.p2, q.p3, q.p4); }
    getBounds() {
      const xs = [this.p1, this.p2, this.p3, this.p4].map((p) => p.x), ys = [this.p1, this.p2, this.p3, this.p4].map((p) => p.y);
      const x = Math.min(...xs), y = Math.min(...ys);
      return new DOMRect(x, y, Math.max(...xs) - x, Math.max(...ys) - y);
    }
    toJSON() { return { p1: this.p1, p2: this.p2, p3: this.p3, p4: this.p4 }; }
  }

  // Column-major 4×4, as the spec's m11…m44 (m41/m42 = translation).
  const ID = () => [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  const NAMES = ["m11", "m12", "m13", "m14", "m21", "m22", "m23", "m24", "m31", "m32", "m33", "m34", "m41", "m42", "m43", "m44"];
  function mul4(a, b) {
    const o = new Array(16);
    for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
      o[c * 4 + r] = s;
    }
    return o;
  }
  function from6(a, b, c, d, e, f) { return [a, b, 0, 0, c, d, 0, 0, 0, 0, 1, 0, e, f, 0, 1]; }
  function parseTransformList(s) {
    s = String(s).trim();
    if (s === "" || s === "none") return [ID(), true];
    let m = ID(), is2D = true;
    const re = /(\w+)\(([^)]*)\)/g;
    let match, consumed = "";
    while ((match = re.exec(s))) {
      consumed += match[0];
      const fn = match[1].toLowerCase();
      const args = match[2].split(/[\s,]+/).filter(Boolean);
      const n = (i, unit) => {
        const v = args[i];
        if (v === undefined) return undefined;
        const f = parseFloat(v);
        if (unit === "angle") return /rad$/.test(v) ? f : /turn$/.test(v) ? f * 2 * Math.PI : /grad$/.test(v) ? (f * Math.PI) / 200 : (f * Math.PI) / 180;
        return f;
      };
      let t;
      switch (fn) {
        case "matrix": t = from6(...[0, 1, 2, 3, 4, 5].map((i) => n(i))); break;
        case "matrix3d": t = args.map(parseFloat); is2D = false; break;
        case "translate": t = from6(1, 0, 0, 1, n(0), n(1) ?? 0); break;
        case "translatex": t = from6(1, 0, 0, 1, n(0), 0); break;
        case "translatey": t = from6(1, 0, 0, 1, 0, n(0)); break;
        case "scale": t = from6(n(0), 0, 0, n(1) ?? n(0), 0, 0); break;
        case "scalex": t = from6(n(0), 0, 0, 1, 0, 0); break;
        case "scaley": t = from6(1, 0, 0, n(0), 0, 0); break;
        case "rotate": { const a = n(0, "angle"); t = from6(Math.cos(a), Math.sin(a), -Math.sin(a), Math.cos(a), 0, 0); break; }
        case "skewx": t = from6(1, 0, Math.tan(n(0, "angle")), 1, 0, 0); break;
        case "skewy": t = from6(1, Math.tan(n(0, "angle")), 0, 1, 0, 0); break;
        default: throw new DOMException("bad transform: " + fn, "SyntaxError");
      }
      if (t.length !== 16 || t.some((v) => !isFinite(v))) throw new DOMException("bad transform", "SyntaxError");
      m = mul4(m, t);
    }
    if (!consumed) throw new DOMException("bad transform: " + s, "SyntaxError");
    return [m, is2D];
  }

  class DOMMatrixReadOnly {
    constructor(init) {
      this.__m = ID(); this.__2d = true;
      if (init === undefined) return;
      if (typeof init === "string") { [this.__m, this.__2d] = parseTransformList(init); return; }
      const a = Array.from(init, Number);
      if (a.length === 6) this.__m = from6(...a);
      else if (a.length === 16) { this.__m = a; this.__2d = false; }
      else throw new TypeError("DOMMatrix: needs 6 or 16 numbers");
    }
    static fromMatrix(o = {}) {
      if (o instanceof DOMMatrixReadOnly) { const r = new this(); r.__m = o.__m.slice(); r.__2d = o.__2d; return r; }
      const r = new this();
      const is3d = NAMES.some((k) => !["m11", "m12", "m21", "m22", "m41", "m42", "m33", "m44"].includes(k) && o[k] !== undefined && o[k] !== 0) || (o.m33 !== undefined && o.m33 !== 1) || (o.m44 !== undefined && o.m44 !== 1);
      const v = (k, alt, d) => num(o[k] !== undefined ? o[k] : o[alt], d);
      r.__m = [v("m11", "a", 1), v("m12", "b", 0), num(o.m13, 0), num(o.m14, 0), v("m21", "c", 0), v("m22", "d", 1), num(o.m23, 0), num(o.m24, 0),
        num(o.m31, 0), num(o.m32, 0), num(o.m33, 1), num(o.m34, 0), v("m41", "e", 0), v("m42", "f", 0), num(o.m43, 0), num(o.m44, 1)];
      r.__2d = o.is2D !== undefined ? !!o.is2D : !is3d;
      return r;
    }
    static fromFloat32Array(a) { return new this(Array.from(a)); }
    static fromFloat64Array(a) { return new this(Array.from(a)); }
    get a() { return this.__m[0]; } get b() { return this.__m[1]; } get c() { return this.__m[4]; }
    get d() { return this.__m[5]; } get e() { return this.__m[12]; } get f() { return this.__m[13]; }
    get is2D() { return this.__2d; }
    get isIdentity() { return this.__m.every((v, i) => v === ID()[i]); }
    multiply(o) { return new DOMMatrix(this).multiplySelf(o); }
    translate(x, y, z) { return new DOMMatrix(this).translateSelf(x, y, z); }
    scale(sx, sy, sz, ox, oy, oz) { return new DOMMatrix(this).scaleSelf(sx, sy, sz, ox, oy, oz); }
    scaleNonUniform(sx = 1, sy = 1) { return new DOMMatrix(this).scaleSelf(sx, sy); }
    scale3d(s, ox, oy, oz) { return new DOMMatrix(this).scale3dSelf(s, ox, oy, oz); }
    rotate(x, y, z) { return new DOMMatrix(this).rotateSelf(x, y, z); }
    rotateFromVector(x, y) { return new DOMMatrix(this).rotateFromVectorSelf(x, y); }
    rotateAxisAngle(x, y, z, a) { return new DOMMatrix(this).rotateAxisAngleSelf(x, y, z, a); }
    skewX(a) { return new DOMMatrix(this).skewXSelf(a); }
    skewY(a) { return new DOMMatrix(this).skewYSelf(a); }
    flipX() { return this.multiply(new DOMMatrix([-1, 0, 0, 1, 0, 0])); }
    flipY() { return this.multiply(new DOMMatrix([1, 0, 0, -1, 0, 0])); }
    inverse() { return new DOMMatrix(this).invertSelf(); }
    transformPoint(p = {}) {
      const m = this.__m, x = num(p.x, 0), y = num(p.y, 0), z = num(p.z, 0), w = num(p.w, 1);
      return new DOMPoint(m[0] * x + m[4] * y + m[8] * z + m[12] * w, m[1] * x + m[5] * y + m[9] * z + m[13] * w,
        m[2] * x + m[6] * y + m[10] * z + m[14] * w, m[3] * x + m[7] * y + m[11] * z + m[15] * w);
    }
    toFloat32Array() { return new Float32Array(this.__m); }
    toFloat64Array() { return new Float64Array(this.__m); }
    toString() {
      if (this.__m.some((v) => !isFinite(v))) throw new DOMException("non-finite matrix", "InvalidStateError");
      return this.__2d ? `matrix(${[this.a, this.b, this.c, this.d, this.e, this.f].join(", ")})` : `matrix3d(${this.__m.join(", ")})`;
    }
    toJSON() { const o = { a: this.a, b: this.b, c: this.c, d: this.d, e: this.e, f: this.f, is2D: this.is2D, isIdentity: this.isIdentity }; NAMES.forEach((k, i) => (o[k] = this.__m[i])); return o; }
  }
  NAMES.forEach((k, i) => Object.defineProperty(DOMMatrixReadOnly.prototype, k, { get() { return this.__m[i]; }, configurable: true }));

  class DOMMatrix extends DOMMatrixReadOnly {
    multiplySelf(o) { const b = DOMMatrixReadOnly.fromMatrix(o); this.__m = mul4(this.__m, b.__m); this.__2d = this.__2d && b.__2d; return this; }
    preMultiplySelf(o) { const b = DOMMatrixReadOnly.fromMatrix(o); this.__m = mul4(b.__m, this.__m); this.__2d = this.__2d && b.__2d; return this; }
    translateSelf(x = 0, y = 0, z = 0) {
      const t = ID(); t[12] = Number(x); t[13] = Number(y); t[14] = Number(z);
      this.__m = mul4(this.__m, t); if (z) this.__2d = false; return this;
    }
    scaleSelf(sx = 1, sy, sz = 1, ox = 0, oy = 0, oz = 0) {
      if (sy === undefined) sy = sx;
      this.translateSelf(ox, oy, oz);
      const t = ID(); t[0] = Number(sx); t[5] = Number(sy); t[10] = Number(sz);
      this.__m = mul4(this.__m, t); if (sz !== 1 || oz) this.__2d = false;
      return this.translateSelf(-ox, -oy, -oz);
    }
    scale3dSelf(s = 1, ox = 0, oy = 0, oz = 0) { return this.scaleSelf(s, s, s, ox, oy, oz); }
    rotateSelf(rx = 0, ry, rz) {
      if (ry === undefined && rz === undefined) { rz = rx; rx = 0; ry = 0; }
      ry = ry || 0; rz = rz || 0;
      if (rz) this.rotateAxisAngleSelf(0, 0, 1, rz);
      if (ry) this.rotateAxisAngleSelf(0, 1, 0, ry);
      if (rx) this.rotateAxisAngleSelf(1, 0, 0, rx);
      return this;
    }
    rotateFromVectorSelf(x = 0, y = 0) { return this.rotateAxisAngleSelf(0, 0, 1, x === 0 && y === 0 ? 0 : (Math.atan2(y, x) * 180) / Math.PI); }
    rotateAxisAngleSelf(x = 0, y = 0, z = 0, angle = 0) {
      const len = Math.hypot(x, y, z);
      if (!len) return this;
      [x, y, z] = [x / len, y / len, z / len];
      const a = (Number(angle) * Math.PI) / 180, s = Math.sin(a / 2), sc = s * Math.cos(a / 2), sq = s * s;
      const t = [
        1 - 2 * (y * y + z * z) * sq, 2 * (x * y * sq + z * sc), 2 * (x * z * sq - y * sc), 0,
        2 * (x * y * sq - z * sc), 1 - 2 * (x * x + z * z) * sq, 2 * (y * z * sq + x * sc), 0,
        2 * (x * z * sq + y * sc), 2 * (y * z * sq - x * sc), 1 - 2 * (x * x + y * y) * sq, 0,
        0, 0, 0, 1,
      ];
      this.__m = mul4(this.__m, t);
      if (x || y) this.__2d = false;
      return this;
    }
    skewXSelf(a = 0) { const t = ID(); t[4] = Math.tan((a * Math.PI) / 180); this.__m = mul4(this.__m, t); return this; }
    skewYSelf(a = 0) { const t = ID(); t[1] = Math.tan((a * Math.PI) / 180); this.__m = mul4(this.__m, t); return this; }
    invertSelf() {
      const m = this.__m, inv = new Array(16);
      inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
      inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
      inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
      inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
      inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
      inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
      inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
      inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
      inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
      inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
      inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
      inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
      inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
      inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
      inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
      inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
      const det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
      if (!det) { this.__m = new Array(16).fill(NaN); this.__2d = false; return this; }
      this.__m = inv.map((v) => v / det);
      return this;
    }
    setMatrixValue(s) { [this.__m, this.__2d] = parseTransformList(s); return this; }
  }
  for (const [k, i] of [["a", 0], ["b", 1], ["c", 4], ["d", 5], ["e", 12], ["f", 13]].concat(NAMES.map((n, i) => [n, i]))) {
    const get = Object.getOwnPropertyDescriptor(DOMMatrixReadOnly.prototype, k).get;
    Object.defineProperty(DOMMatrix.prototype, k, {
      get, configurable: true,
      set(v) { this.__m[i] = Number(v); if (![0, 1, 4, 5, 12, 13, 10, 15].includes(i) && Number(v) !== 0) this.__2d = false; },
    });
  }

  // Keep any of these another part of the browser already defined.
  for (const [k, v] of Object.entries({ DOMPointReadOnly, DOMPoint, DOMRectReadOnly, DOMRect, DOMQuad, DOMMatrixReadOnly, DOMMatrix })) {
    if (typeof g[k] !== "function") g[k] = v;
  }
  if (typeof g.WebKitCSSMatrix !== "function") g.WebKitCSSMatrix = DOMMatrix;
})();
