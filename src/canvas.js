// <canvas> 2D API: CanvasRenderingContext2D, Path2D, CanvasGradient, CanvasPattern, ImageData,
// OffscreenCanvas. The drawing state and the current path live here; canvas.rs rasterizes.
// Paths are kept in canvas pixels (points are transformed as they're added, as the spec says).
(function () {
  const g = globalThis;
  if (typeof g.CanvasRenderingContext2D === "function") return;
  // The hidden canvases behind OffscreenCanvas, ImageBitmap and copies. Kept from setup:
  // a worker loses `document` afterwards (worker_scope.js) but has OffscreenCanvas, and pages
  // run it there (a "document is not defined" error from dickinsonhistoricalsociety.org).
  const setupDocument = g.document;
  const hiddenCanvas = () => setupDocument.createElement("canvas");

  const OPS = {
    "source-over": 0, "source-in": 1, "source-out": 2, "source-atop": 3, "destination-over": 4, "destination-in": 5,
    "destination-out": 6, "destination-atop": 7, lighter: 8, copy: 9, xor: 10, multiply: 11, screen: 12, overlay: 13,
    darken: 14, lighten: 15, "color-dodge": 16, "color-burn": 17, "hard-light": 18, "soft-light": 19, difference: 20,
    exclusion: 21, hue: 22, saturation: 23, color: 24, luminosity: 25, clear: 26,
  };
  const CAPS = { butt: 0, round: 1, square: 2 };
  const JOINS = { miter: 0, round: 1, bevel: 2 };
  const ALIGNS = { start: 0, left: 0, center: 1, right: 2, end: 2 };
  const BASELINES = { alphabetic: 0, top: 1, hanging: 2, middle: 3, bottom: 4, ideographic: 5 };
  const TAU = Math.PI * 2;

  // ---- colors (parsed by the browser's own CSS color parser)
  const colorCache = new Map();
  // rgb()/hsl() in the exact CSS syntax (stylesheets are more forgiving than canvas must be).
  const NUM = "[+-]?(?:\\d+\\.?\\d*|\\.\\d+)(?:e[+-]?\\d+)?";
  const re = (body) => new RegExp("^(?:" + body + ")$", "i");
  const R_NUM = re(NUM), R_PCT = re(NUM + "%"), R_NP = re(NUM + "|" + NUM + "%"), R_NPN = re(NUM + "|" + NUM + "%|none");
  const R_ANG = re(NUM + "(?:deg|rad|grad|turn)?"), R_ANGN = re(NUM + "(?:deg|rad|grad|turn)?|none");
  function strictColor(s) {
    const m = /^(rgba?|hsla?)\(([\s\S]*)\)$/i.exec(s);
    if (!m) return true;
    const hsl = m[1].toLowerCase().startsWith("hsl"), body = m[2].trim();
    if (body.includes(",")) {
      const p = body.split(",").map((x) => x.trim());
      if (p.length !== 3 && p.length !== 4) return false;
      if (p.length === 4 && !R_NP.test(p[3])) return false;
      if (hsl) return R_ANG.test(p[0]) && R_PCT.test(p[1]) && R_PCT.test(p[2]);
      const three = p.slice(0, 3);
      return three.every((x) => R_NUM.test(x)) || three.every((x) => R_PCT.test(x));
    }
    const parts = body.split("/").map((x) => x.trim());
    if (parts.length > 2 || (parts.length === 2 && !R_NPN.test(parts[1]))) return false;
    const v = parts[0].split(/\s+/);
    if (v.length !== 3) return false;
    return hsl ? R_ANGN.test(v[0]) && R_NPN.test(v[1]) && R_NPN.test(v[2]) : v.every((x) => R_NPN.test(x));
  }
  function parseColor(s) {
    s = String(s).trim();
    if (colorCache.has(s)) return colorCache.get(s);
    const c = strictColor(s) ? __cvColor(s) : null;
    const out = c ? [c[0] / 255, c[1] / 255, c[2] / 255, c[3] / 255] : null;
    if (colorCache.size > 500) colorCache.clear();
    colorCache.set(s, out);
    return out;
  }
  function serializeColor(c) {
    const r = Math.round(c[0] * 255), gg = Math.round(c[1] * 255), b = Math.round(c[2] * 255);
    if (c[3] >= 1) return "#" + [r, gg, b].map((v) => v.toString(16).padStart(2, "0")).join("");
    // The shortest alpha that maps back to the same byte (128 → 0.5).
    const a8 = Math.round(c[3] * 255);
    let a = Math.round((a8 / 255) * 100) / 100;
    if (Math.round(a * 255) !== a8) a = Math.round((a8 / 255) * 1000) / 1000;
    return `rgba(${r}, ${gg}, ${b}, ${a})`;
  }

  // ---- matrices [a b c d e f]
  const mul = (m, n) => [
    m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
    m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
    m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5],
  ];
  const apply = (m, x, y) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
  function invert(m) {
    const det = m[0] * m[3] - m[1] * m[2];
    if (!det || !isFinite(det)) return null;
    return [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det, (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det];
  }
  const fin = (...v) => v.map(Number).every(isFinite);

  // ---- paths: commands 0 move · 1 line · 2 quad · 3 cubic · 4 close, in some coordinate space
  class PathData {
    constructor() { this.c = []; this.start = null; this.last = null; this.segs = 0; }
    copy() { const p = new PathData(); p.c = this.c.slice(); p.start = this.start && this.start.slice(); p.last = this.last && this.last.slice(); p.segs = this.segs; return p; }
    move(m, x, y) { const [X, Y] = apply(m, x, y); this.c.push(0, X, Y); this.start = [X, Y]; this.last = [X, Y]; this.segs = 0; }
    ensure(m, x, y) { if (!this.last) this.move(m, x, y); }
    // Zero-length segments are dropped (so they draw no line caps).
    same(...pts) { const l = this.last; for (let i = 0; i < pts.length; i += 2) if (pts[i] !== l[0] || pts[i + 1] !== l[1]) return false; return true; }
    line(m, x, y) {
      if (!this.last) return this.move(m, x, y);
      const [X, Y] = apply(m, x, y);
      if (this.same(X, Y)) return;
      this.c.push(1, X, Y); this.last = [X, Y]; this.segs++;
    }
    quad(m, cx, cy, x, y) {
      this.ensure(m, cx, cy);
      const [a, b] = apply(m, cx, cy), [X, Y] = apply(m, x, y);
      if (this.same(a, b, X, Y)) return;
      this.c.push(2, a, b, X, Y); this.last = [X, Y]; this.segs++;
    }
    cubic(m, c1x, c1y, c2x, c2y, x, y) {
      this.ensure(m, c1x, c1y);
      const [a, b] = apply(m, c1x, c1y), [c, d] = apply(m, c2x, c2y), [X, Y] = apply(m, x, y);
      if (this.same(a, b, c, d, X, Y)) return;
      this.c.push(3, a, b, c, d, X, Y); this.last = [X, Y]; this.segs++;
    }
    close() {
      if (!this.last) return;
      if (this.segs) this.c.push(4);
      this.last = this.start && this.start.slice();
    }
    // Polylines (curves flattened), one per subpath: [[x, y, x, y, …], closed].
    polys() {
      const out = [];
      let cur = null;
      const c = this.c;
      for (let i = 0; i < c.length;) {
        const op = c[i];
        if (op === 0) { cur = [[c[i + 1], c[i + 2]], false]; out.push(cur); i += 3; continue; }
        if (!cur) { i += op === 4 ? 1 : op === 2 ? 5 : op === 3 ? 7 : 3; continue; }
        const pts = cur[0], x0 = pts[pts.length - 2], y0 = pts[pts.length - 1];
        if (op === 1) { pts.push(c[i + 1], c[i + 2]); i += 3; }
        else if (op === 2 || op === 3) {
          const n = 16;
          for (let k = 1; k <= n; k++) {
            const t = k / n, u = 1 - t;
            if (op === 2) pts.push(u * u * x0 + 2 * u * t * c[i + 1] + t * t * c[i + 3], u * u * y0 + 2 * u * t * c[i + 2] + t * t * c[i + 4]);
            else pts.push(u * u * u * x0 + 3 * u * u * t * c[i + 1] + 3 * u * t * t * c[i + 3] + t * t * t * c[i + 5], u * u * u * y0 + 3 * u * u * t * c[i + 2] + 3 * u * t * t * c[i + 4] + t * t * t * c[i + 6]);
          }
          i += op === 2 ? 5 : 7;
        } else { cur[1] = true; cur = [[pts[0], pts[1]], false]; out.push(cur); i += 1; }
      }
      return out;
    }
    rect(m, x, y, w, h) {
      this.move(m, x, y); this.line(m, x + w, y); this.line(m, x + w, y + h); this.line(m, x, y + h); this.close();
      this.move(m, x, y);
    }
    // An elliptical arc as cubic Béziers (each covering at most a quarter turn).
    ellipse(m, x, y, rx, ry, rot, a0, a1, ccw) {
      // The end angle as other browsers settle it (a whole turn when start and end differ
      // by a multiple of 2π in the "wrong" direction).
      let end = a1;
      if (!ccw && a1 - a0 >= TAU) end = a0 + TAU;
      else if (ccw && a0 - a1 >= TAU) end = a0 - TAU;
      else if (!ccw && a0 > a1) end = a0 + (TAU - ((a0 - a1) % TAU));
      else if (ccw && a0 < a1) end = a0 - (TAU - ((a1 - a0) % TAU));
      const sweep = end - a0;
      const cos = Math.cos(rot), sin = Math.sin(rot);
      const pt = (t) => {
        const ex = rx * Math.cos(t), ey = ry * Math.sin(t);
        return [x + ex * cos - ey * sin, y + ex * sin + ey * cos];
      };
      const d = (t) => {
        const ex = -rx * Math.sin(t), ey = ry * Math.cos(t);
        return [ex * cos - ey * sin, ex * sin + ey * cos];
      };
      const p0 = pt(a0);
      if (this.last) this.line(m, p0[0], p0[1]); else this.move(m, p0[0], p0[1]);
      if (sweep === 0) return;
      const n = Math.max(1, Math.ceil(Math.abs(sweep) / (Math.PI / 4) - 1e-9));
      const step = sweep / n;
      const k = (4 / 3) * Math.tan(step / 4);
      let t = a0;
      for (let i = 0; i < n; i++) {
        const t2 = t + step;
        const s = pt(t), e = pt(t2), ds = d(t), de = d(t2);
        this.cubic(m, s[0] + k * ds[0], s[1] + k * ds[1], e[0] - k * de[0], e[1] - k * de[1], e[0], e[1]);
        t = t2;
      }
    }
    // Transformed copy (for Path2D drawn under the current transform).
    mapped(m) {
      const out = new PathData();
      const c = this.c;
      for (let i = 0; i < c.length;) {
        const op = c[i];
        const n = op === 0 || op === 1 ? 2 : op === 2 ? 4 : op === 3 ? 6 : 0;
        out.c.push(op);
        for (let k = 0; k < n; k += 2) { const [X, Y] = apply(m, c[i + 1 + k], c[i + 2 + k]); out.c.push(X, Y); }
        i += 1 + n;
      }
      return out;
    }
  }

  // Shared path-building methods (CanvasRenderingContext2D and Path2D): `this.__path` and `this.__m()`.
  const pathMethods = {
    moveTo(x, y) { if (fin(x, y)) this.__path.move(this.__m(), +x, +y); },
    lineTo(x, y) { if (fin(x, y)) this.__path.line(this.__m(), +x, +y); },
    quadraticCurveTo(cx, cy, x, y) { if (fin(cx, cy, x, y)) this.__path.quad(this.__m(), +cx, +cy, +x, +y); },
    bezierCurveTo(a, b, c, d, x, y) { if (fin(a, b, c, d, x, y)) this.__path.cubic(this.__m(), +a, +b, +c, +d, +x, +y); },
    closePath() { this.__path.close(); },
    rect(x, y, w, h) { if (fin(x, y, w, h)) this.__path.rect(this.__m(), +x, +y, +w, +h); },
    roundRect(x, y, w, h, radii = 0) {
      if (!fin(x, y, w, h)) return;
      x = +x; y = +y; w = +w; h = +h;
      const list = radii !== null && typeof radii === "object" && typeof radii[Symbol.iterator] === "function" ? Array.from(radii) : [radii];
      if (list.length < 1 || list.length > 4) throw new RangeError("roundRect: between one and four radii are needed");
      const rs = [];
      for (const r of list) {
        if (r !== null && typeof r === "object") {
          const rx = +(r.x ?? 0), ry = +(r.y ?? 0);
          if (!isFinite(rx) || !isFinite(ry)) return;
          if (rx < 0 || ry < 0) throw new RangeError("roundRect: radii must be non-negative");
          rs.push([rx, ry]);
        } else {
          const v = +r;
          if (!isFinite(v)) return;
          if (v < 0) throw new RangeError("roundRect: radii must be non-negative");
          rs.push([v, v]);
        }
      }
      let [ul, ur, lr, ll] = rs.length === 1 ? [rs[0], rs[0], rs[0], rs[0]] : rs.length === 2 ? [rs[0], rs[1], rs[0], rs[1]] : rs.length === 3 ? [rs[0], rs[1], rs[2], rs[1]] : rs;
      const W = Math.abs(w), Hh = Math.abs(h);
      const top = ul[0] + ur[0], right = ur[1] + lr[1], bottom = lr[0] + ll[0], left = ul[1] + ll[1];
      const f = Math.min(1, top ? W / top : 1, right ? Hh / right : 1, bottom ? W / bottom : 1, left ? Hh / left : 1);
      [ul, ur, lr, ll] = [ul, ur, lr, ll].map(([a, b]) => [a * f, b * f]);
      // Drawn in a frame where width and height are positive; a negative width or height
      // mirrors it, which also reverses the winding (as other browsers do).
      const sx = w < 0 ? -1 : 1, sy = h < 0 ? -1 : 1;
      const m0 = this.__m(), m = mul(m0, [sx, 0, 0, sy, x, y]), p = this.__path, H = Math.PI / 2;
      p.move(m, ul[0], 0);
      p.line(m, W - ur[0], 0);
      if (ur[0] && ur[1]) p.ellipse(m, W - ur[0], ur[1], ur[0], ur[1], 0, -H, 0, false);
      p.line(m, W, Hh - lr[1]);
      if (lr[0] && lr[1]) p.ellipse(m, W - lr[0], Hh - lr[1], lr[0], lr[1], 0, 0, H, false);
      p.line(m, ll[0], Hh);
      if (ll[0] && ll[1]) p.ellipse(m, ll[0], Hh - ll[1], ll[0], ll[1], 0, H, Math.PI, false);
      p.line(m, 0, ul[1]);
      if (ul[0] && ul[1]) p.ellipse(m, ul[0], ul[1], ul[0], ul[1], 0, Math.PI, 3 * H, false);
      p.close();
      p.move(m0, x, y);
    },
    arc(x, y, r, a0, a1, ccw = false) {
      if (!fin(x, y, r, a0, a1)) return;
      if (r < 0) throw new DOMException("arc: negative radius", "IndexSizeError");
      this.__path.ellipse(this.__m(), +x, +y, +r, +r, 0, +a0, +a1, !!ccw);
    },
    ellipse(x, y, rx, ry, rot, a0, a1, ccw = false) {
      if (!fin(x, y, rx, ry, rot, a0, a1)) return;
      if (rx < 0 || ry < 0) throw new DOMException("ellipse: negative radius", "IndexSizeError");
      this.__path.ellipse(this.__m(), +x, +y, +rx, +ry, +rot, +a0, +a1, !!ccw);
    },
    arcTo(x1, y1, x2, y2, r) {
      if (!fin(x1, y1, x2, y2, r)) return;
      if (r < 0) throw new DOMException("arcTo: negative radius", "IndexSizeError");
      const m = this.__m(), p = this.__path;
      if (!p.last) return p.move(m, x1, y1);
      const inv = invert(m);
      if (!inv) return;
      const [x0, y0] = apply(inv, p.last[0], p.last[1]);
      const v1x = x0 - x1, v1y = y0 - y1, v2x = x2 - x1, v2y = y2 - y1;
      const l1 = Math.hypot(v1x, v1y), l2 = Math.hypot(v2x, v2y);
      const cross = v1x * v2y - v1y * v2x;
      if (!l1 || !l2 || Math.abs(cross) < 1e-9 || r === 0) return p.line(m, x1, y1);
      const cosT = (v1x * v2x + v1y * v2y) / (l1 * l2);
      const half = Math.acos(Math.max(-1, Math.min(1, cosT))) / 2;
      const dist = r / Math.tan(half);
      const t1x = x1 + (v1x / l1) * dist, t1y = y1 + (v1y / l1) * dist;
      const t2x = x1 + (v2x / l2) * dist, t2y = y1 + (v2y / l2) * dist;
      // Center: from the corner along the bisector.
      const bx = v1x / l1 + v2x / l2, by = v1y / l1 + v2y / l2, bl = Math.hypot(bx, by);
      const cd = r / Math.sin(half);
      const cx = x1 + (bx / bl) * cd, cy = y1 + (by / bl) * cd;
      const a0 = Math.atan2(t1y - cy, t1x - cx), a1 = Math.atan2(t2y - cy, t2x - cx);
      p.line(m, t1x, t1y);
      p.ellipse(m, cx, cy, r, r, 0, a0, a1, cross > 0);
    },
  };

  // Winding-number test; points exactly on an edge count as inside.
  function pointInPolys(polys, x, y, evenOdd) {
    let wind = 0, crossings = 0;
    for (const [pts] of polys) {
      const n = pts.length / 2;
      if (n < 2) continue;
      for (let i = 0; i < n; i++) {
        const x0 = pts[2 * i], y0 = pts[2 * i + 1];
        const j = (i + 1) % n, x1 = pts[2 * j], y1 = pts[2 * j + 1];
        const dx = x1 - x0, dy = y1 - y0, len2 = dx * dx + dy * dy;
        if (len2 > 0) {
          const t = Math.max(0, Math.min(1, ((x - x0) * dx + (y - y0) * dy) / len2));
          if (Math.hypot(x0 + t * dx - x, y0 + t * dy - y) < 1e-6) return true;
        }
        if ((y0 <= y) !== (y1 <= y)) {
          const xi = x0 + ((y - y0) / (y1 - y0)) * (x1 - x0);
          if (xi > x) { wind += y1 > y0 ? 1 : -1; crossings++; }
        }
      }
    }
    return evenOdd ? crossings % 2 === 1 : wind !== 0;
  }
  // Is (x, y) within half the line width of the path, on a dash that's drawn?
  function pointOnStroke(polys, x, y, s) {
    const half = s.lineWidth / 2, dash = s.dash, total = dash.reduce((a, b) => a + b, 0);
    const drawn = (dist) => {
      if (!dash.length || !total) return true;
      let d = (((dist + s.dashOffset) % total) + total) % total;
      for (let i = 0; i < dash.length; i++) { if (d < dash[i]) return i % 2 === 0; d -= dash[i]; }
      return true;
    };
    for (const [pts, closed] of polys) {
      const n = pts.length / 2;
      let along = 0;
      const segs = closed ? n : n - 1;
      for (let i = 0; i < segs; i++) {
        const j = (i + 1) % n;
        const x0 = pts[2 * i], y0 = pts[2 * i + 1], x1 = pts[2 * j], y1 = pts[2 * j + 1];
        const dx = x1 - x0, dy = y1 - y0, len = Math.hypot(dx, dy);
        if (len > 0) {
          const raw = ((x - x0) * dx + (y - y0) * dy) / (len * len);
          const t = Math.max(0, Math.min(1, raw));
          const dist = Math.hypot(x0 + t * dx - x, y0 + t * dy - y);
          if (dist <= half) {
            const at = along + t * len;
            if (raw >= 0 && raw <= 1) {
              if (drawn(at)) return true;
            } else {
              // Beyond a segment's end: a join if the line carries on there (and the dash
              // covers it), a cap at an open end (round and square caps only).
              const openEnd = !closed && ((raw < 0 && i === 0) || (raw > 1 && i === segs - 1));
              if (openEnd) {
                if (s.cap !== "butt" && drawn(at)) return true;
              } else {
                // Only a real corner adds area (a flattened curve's tiny bends don't).
                const k = raw > 1 ? (j + 1) % n : (i - 1 + n) % n;
                const v = raw > 1 ? j : i;
                const ex = pts[2 * k] - pts[2 * v], ey = pts[2 * k + 1] - pts[2 * v + 1];
                const el = Math.hypot(ex, ey) || 1;
                const cos = raw > 1 ? (dx * ex + dy * ey) / (len * el) : -(dx * ex + dy * ey) / (len * el);
                if (cos < 0.97 && drawn(at - 1e-3) && drawn(at + 1e-3)) return true;
              }
            }
          }
        }
        along += len;
      }
    }
    return false;
  }

  class Path2D {
    constructor(src) {
      this.__path = new PathData();
      if (src instanceof Path2D) this.__path = src.__path.copy();
      else if (typeof src === "string") parseSvgPath(this, src);
    }
    __m() { return [1, 0, 0, 1, 0, 0]; }
    addPath(p, t) {
      if (!(p instanceof Path2D)) throw new TypeError("addPath: not a Path2D");
      const d = t ? DOMMatrix.fromMatrix(t) : null;
      const m = d ? [d.a, d.b, d.c, d.d, d.e, d.f] : [1, 0, 0, 1, 0, 0];
      const q = p.__path.mapped(m);
      this.__path.c.push(...q.c);
      this.__path.last = q.c.length ? [q.c[q.c.length - 2], q.c[q.c.length - 1]] : this.__path.last;
    }
  }
  Object.assign(Path2D.prototype, pathMethods);

  // SVG path data (M L H V C S Q T A Z, absolute and relative) for new Path2D("…").
  function parseSvgPath(p, d) {
    const toks = String(d).match(/[a-df-z]|[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?/gi) || [];
    let i = 0, cmd = "", cx = 0, cy = 0, sx = 0, sy = 0, lcx = 0, lcy = 0, lq = null;
    const nextNum = () => parseFloat(toks[i++]);
    const isNum = () => i < toks.length && !/[a-z]/i.test(toks[i]);
    while (i < toks.length) {
      if (/[a-z]/i.test(toks[i])) cmd = toks[i++];
      else if (!cmd) break;
      const rel = cmd === cmd.toLowerCase() && cmd !== "z" ? 1 : 0;
      const C = cmd.toUpperCase();
      const ox = rel ? cx : 0, oy = rel ? cy : 0;
      if (C === "Z") { p.closePath(); cx = sx; cy = sy; lq = null; lcx = cx; lcy = cy; if (!isNum()) { cmd = ""; } continue; }
      if (!isNum()) break;
      if (C === "M") { cx = ox + nextNum(); cy = oy + nextNum(); p.moveTo(cx, cy); sx = cx; sy = cy; cmd = rel ? "l" : "L"; lcx = cx; lcy = cy; lq = null; }
      else if (C === "L") { cx = ox + nextNum(); cy = oy + nextNum(); p.lineTo(cx, cy); lcx = cx; lcy = cy; lq = null; }
      else if (C === "H") { cx = ox + nextNum(); p.lineTo(cx, cy); lcx = cx; lcy = cy; lq = null; }
      else if (C === "V") { cy = (rel ? cy : 0) + nextNum(); p.lineTo(cx, cy); lcx = cx; lcy = cy; lq = null; }
      else if (C === "C") {
        const a = ox + nextNum(), b = oy + nextNum(), c = ox + nextNum(), e = oy + nextNum();
        cx = ox + nextNum(); cy = oy + nextNum(); p.bezierCurveTo(a, b, c, e, cx, cy); lcx = c; lcy = e; lq = null;
      } else if (C === "S") {
        const a = 2 * cx - lcx, b = 2 * cy - lcy, c = ox + nextNum(), e = oy + nextNum();
        cx = ox + nextNum(); cy = oy + nextNum(); p.bezierCurveTo(a, b, c, e, cx, cy); lcx = c; lcy = e; lq = null;
      } else if (C === "Q") {
        const a = ox + nextNum(), b = oy + nextNum();
        cx = ox + nextNum(); cy = oy + nextNum(); p.quadraticCurveTo(a, b, cx, cy); lq = [a, b]; lcx = cx; lcy = cy;
      } else if (C === "T") {
        const q = lq ? [2 * cx - lq[0], 2 * cy - lq[1]] : [cx, cy];
        cx = ox + nextNum(); cy = oy + nextNum(); p.quadraticCurveTo(q[0], q[1], cx, cy); lq = q; lcx = cx; lcy = cy;
      } else if (C === "A") {
        let rx = Math.abs(nextNum()), ry = Math.abs(nextNum());
        const rot = (nextNum() * Math.PI) / 180, large = nextNum() !== 0, sweep = nextNum() !== 0;
        const x2 = ox + nextNum(), y2 = oy + nextNum();
        svgArc(p, cx, cy, rx, ry, rot, large, sweep, x2, y2);
        cx = x2; cy = y2; lcx = cx; lcy = cy; lq = null;
      } else break;
    }
  }
  // SVG endpoint arc → center form (SVG spec, appendix F.6).
  function svgArc(p, x1, y1, rx, ry, phi, large, sweep, x2, y2) {
    if (!rx || !ry) return p.lineTo(x2, y2);
    const cos = Math.cos(phi), sin = Math.sin(phi);
    const dx = (x1 - x2) / 2, dy = (y1 - y2) / 2;
    const xp = cos * dx + sin * dy, yp = -sin * dx + cos * dy;
    const lam = (xp * xp) / (rx * rx) + (yp * yp) / (ry * ry);
    if (lam > 1) { rx *= Math.sqrt(lam); ry *= Math.sqrt(lam); }
    const num = rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp;
    let co = Math.sqrt(Math.max(0, num / (rx * rx * yp * yp + ry * ry * xp * xp)));
    if (large === sweep) co = -co;
    const cxp = (co * rx * yp) / ry, cyp = (-co * ry * xp) / rx;
    const cx = cos * cxp - sin * cyp + (x1 + x2) / 2, cy = sin * cxp + cos * cyp + (y1 + y2) / 2;
    const ang = (ux, uy, vx, vy) => Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    const t1 = ang(1, 0, (xp - cxp) / rx, (yp - cyp) / ry);
    let dt = ang((xp - cxp) / rx, (yp - cyp) / ry, (-xp - cxp) / rx, (-yp - cyp) / ry);
    if (!sweep && dt > 0) dt -= TAU; else if (sweep && dt < 0) dt += TAU;
    p.__path.ellipse(p.__m(), cx, cy, rx, ry, phi, t1, t1 + dt, dt < 0);
  }

  class CanvasGradient {
    constructor(spec) { this.__spec = spec; this.__stops = []; }
    addColorStop(offset, color) {
      offset = +offset;
      if (!(offset >= 0 && offset <= 1)) throw new DOMException("addColorStop: offset out of range", "IndexSizeError");
      const c = parseColor(color);
      if (!c) throw new DOMException("addColorStop: bad color", "SyntaxError");
      this.__stops.push([offset, ...c]);
      this.__stops.sort((a, b) => a[0] - b[0]);
    }
    __array() {
      const out = this.__spec.slice();
      out.push(this.__stops.length);
      for (const s of this.__stops) out.push(...s);
      return out;
    }
  }
  class CanvasPattern {
    constructor(src, rep) { this.__src = src; this.__rep = rep; this.__m = null; }
    setTransform(t) {
      if (t === undefined) { this.__m = null; return; }
      const d = DOMMatrix.fromMatrix(t);
      const m = [d.a, d.b, d.c, d.d, d.e, d.f];
      if (m.every(isFinite)) this.__m = m;
    }
  }
  const DATA = Symbol("data");
  class ImageData {
    constructor(a, b, c, d) {
      if (arguments.length < 2) throw new TypeError("ImageData: not enough arguments");
      let data, w, h, settings;
      if (a instanceof Uint8ClampedArray) {
        data = a;
        if (!data.length || data.length % 4) throw new DOMException("ImageData: data length must be a multiple of 4", "InvalidStateError");
        w = b >>> 0;
        if (!w) throw new DOMException("ImageData: zero width", "IndexSizeError");
        const pixels = data.length / 4;
        if (pixels % w) throw new DOMException("ImageData: data doesn't fit the width", "IndexSizeError");
        h = pixels / w;
        if (c !== undefined && (c === null || typeof c !== "object")) {
          if (c >>> 0 !== h) throw new DOMException("ImageData: data doesn't fit the height", "IndexSizeError");
          settings = d;
        } else settings = c;
      } else if (a !== null && typeof a === "object") {
        throw new TypeError("ImageData: data must be a Uint8ClampedArray");
      } else {
        w = a >>> 0; h = b >>> 0; settings = c;
        if (!w || !h) throw new DOMException("ImageData: zero size", "IndexSizeError");
        if (w * h * 4 > 0x7fffffff) throw new RangeError("ImageData: too large");
        data = new Uint8ClampedArray(w * h * 4);
      }
      const fmt = settings && settings.pixelFormat;
      if (fmt !== undefined && fmt !== "rgba-unorm8") throw new TypeError("ImageData: only rgba-unorm8 pixels are supported");
      const cs = settings && settings.colorSpace;
      this[DATA] = { data, width: w, height: h, colorSpace: cs === "display-p3" ? "display-p3" : "srgb", pixelFormat: "rgba-unorm8" };
    }
    get data() { return this[DATA].data; }
    get width() { return this[DATA].width; }
    get height() { return this[DATA].height; }
    get colorSpace() { return this[DATA].colorSpace; }
    get pixelFormat() { return this[DATA].pixelFormat; }
  }
  class TextMetrics {}

  // Where a drawImage / createPattern source's pixels are: [kind, key, width, height].
  function sourceOf(img) {
    if (img == null) throw new TypeError("drawImage: no image");
    if (img.__offscreen) img = img.__offscreen;
    if (img.__bitmap) img = img.__bitmap;
    if (img instanceof ImageData) return null;
    if (typeof img === "object" && img.nodeType === 1) {
      const tag = img.localName;
      if (tag === "canvas") {
        if (g.__glFlushCanvas) g.__glFlushCanvas(img.__id);
        if (!contexts.has(img.__id) && canvasDim(img, "width", 300) && canvasDim(img, "height", 150)) getContext.call(img, "2d");
        const s = __cvSourceSize(1, String(img.__id));
        return s ? [1, String(img.__id), s[0], s[1]] : null;
      }
      if (tag === "img" || tag === "image") {
        const s = __cvSourceSize(0, String(img.__id));
        return s ? [0, String(img.__id), s[0], s[1]] : null;
      }
      if (tag === "video") {
        const s = __cvSourceSize(3, String(img.__id));
        return s ? [3, String(img.__id), s[0], s[1]] : null;
      }
    }
    throw new TypeError("drawImage: unsupported image source");
  }

  function fontParse(s) {
    const m = /^\s*((?:(?:normal|italic|oblique|small-caps|bold|bolder|lighter|[1-9]00|ultra-condensed|extra-condensed|condensed|semi-condensed|semi-expanded|expanded|extra-expanded|ultra-expanded)\s+)*)(\d*\.?\d+)(px|pt|pc|em|rem|%|in|cm|mm)(?:\s*\/\s*[^\s]+)?\s+(.+?)\s*$/i.exec(String(s));
    if (!m) return null;
    const pre = m[1].toLowerCase();
    let size = parseFloat(m[2]);
    const unit = m[3].toLowerCase();
    size = unit === "pt" ? (size * 4) / 3 : unit === "pc" ? size * 16 : unit === "em" || unit === "rem" ? size * 10 : unit === "%" ? size / 10 : unit === "in" ? size * 96 : unit === "cm" ? (size * 96) / 2.54 : unit === "mm" ? (size * 96) / 25.4 : size;
    const bold = /\b(bold|bolder|[6-9]00)\b/.test(pre);
    const italic = /\b(italic|oblique)\b/.test(pre);
    const family = m[4];
    const parts = [];
    if (italic) parts.push("italic");
    if (bold) parts.push("bold");
    parts.push(size + "px", family);
    return { size, bold, italic, family: family.replace(/["']/g, ""), css: parts.join(" ") };
  }

  function defaults() {
    return {
      fill: [0, 0, 0, 1], stroke: [0, 0, 0, 1], alpha: 1, op: "source-over", lineWidth: 1, cap: "butt", join: "miter",
      miter: 10, dash: [], dashOffset: 0, font: fontParse("10px sans-serif"), align: "start", baseline: "alphabetic",
      direction: "inherit", shadowBlur: 0, shadowColor: [0, 0, 0, 0], shadowX: 0, shadowY: 0, smoothing: true,
      smoothingQuality: "low", filter: "none", letterSpacing: "0px", wordSpacing: "0px", m: [1, 0, 0, 1, 0, 0],
    };
  }

  class CanvasRenderingContext2D {
    constructor(canvas) {
      this.__canvas = canvas;
      this.__s = defaults();
      this.__stack = [];
      this.__path = new PathData();
      this.__size = [0, 0];
      this.__init();
    }
    __init() {
      const w = canvasDim(this.__canvas, "width", 300), h = canvasDim(this.__canvas, "height", 150);
      this.__size = [w, h];
      __cvInit(this.__canvas.__id, w, h);
    }
    // A width/height change (attribute or property) clears the canvas and resets its state.
    __check() {
      const w = canvasDim(this.__canvas, "width", 300), h = canvasDim(this.__canvas, "height", 150);
      if (w !== this.__size[0] || h !== this.__size[1]) this.__reset();
    }
    __reset() { this.__s = defaults(); this.__stack = []; this.__path = new PathData(); this.__init(); }
    __m() { return this.__s.m; }
    get canvas() { return this.__canvas.__offscreenOwner || this.__canvas; }
    getContextAttributes() { return { alpha: true, colorSpace: "srgb", desynchronized: false, willReadFrequently: false }; }
    isContextLost() { return false; }
    reset() { this.__reset(); }

    // ---- state
    save() { this.__stack.push({ ...this.__s, dash: this.__s.dash.slice() }); __cvSave(this.__canvas.__id); }
    restore() { if (this.__stack.length) { this.__s = this.__stack.pop(); __cvRestore(this.__canvas.__id); } }
    get globalAlpha() { return this.__s.alpha; }
    set globalAlpha(v) { v = +v; if (v >= 0 && v <= 1) this.__s.alpha = v; }
    get globalCompositeOperation() { return this.__s.op; }
    set globalCompositeOperation(v) { if (v in OPS) this.__s.op = v; }
    get fillStyle() { return styleOut(this.__s.fill); }
    set fillStyle(v) { const s = styleIn(v, this.__canvas); if (s) this.__s.fill = s; }
    get strokeStyle() { return styleOut(this.__s.stroke); }
    set strokeStyle(v) { const s = styleIn(v, this.__canvas); if (s) this.__s.stroke = s; }
    get lineWidth() { return this.__s.lineWidth; }
    set lineWidth(v) { v = +v; if (v > 0 && isFinite(v)) this.__s.lineWidth = v; }
    get lineCap() { return this.__s.cap; }
    set lineCap(v) { if (v in CAPS) this.__s.cap = v; }
    get lineJoin() { return this.__s.join; }
    set lineJoin(v) { if (v in JOINS) this.__s.join = v; }
    get miterLimit() { return this.__s.miter; }
    set miterLimit(v) { v = +v; if (v > 0 && isFinite(v)) this.__s.miter = v; }
    setLineDash(a) {
      const d = Array.from(a, Number);
      if (d.some((x) => !isFinite(x) || x < 0)) return;
      this.__s.dash = d.length % 2 ? d.concat(d) : d;
    }
    getLineDash() { return this.__s.dash.slice(); }
    get lineDashOffset() { return this.__s.dashOffset; }
    set lineDashOffset(v) { v = +v; if (isFinite(v)) this.__s.dashOffset = v; }
    get font() { return this.__s.font.css; }
    set font(v) { const f = fontParse(v); if (f) this.__s.font = f; }
    get textAlign() { return this.__s.align; }
    set textAlign(v) { if (v in ALIGNS) this.__s.align = v; }
    get textBaseline() { return this.__s.baseline; }
    set textBaseline(v) { if (v in BASELINES) this.__s.baseline = v; }
    get direction() { return this.__s.direction; }
    set direction(v) { if (["ltr", "rtl", "inherit"].includes(v)) this.__s.direction = v; }
    get letterSpacing() { return this.__s.letterSpacing; }
    set letterSpacing(v) { this.__s.letterSpacing = String(v); }
    get wordSpacing() { return this.__s.wordSpacing; }
    set wordSpacing(v) { this.__s.wordSpacing = String(v); }
    get shadowBlur() { return this.__s.shadowBlur; }
    set shadowBlur(v) { v = +v; if (v >= 0 && isFinite(v)) this.__s.shadowBlur = v; }
    get shadowColor() { return serializeColor(this.__s.shadowColor); }
    set shadowColor(v) { const c = styleIn(String(v), this.__canvas); if (Array.isArray(c)) this.__s.shadowColor = c; }
    get shadowOffsetX() { return this.__s.shadowX; }
    set shadowOffsetX(v) { v = +v; if (isFinite(v)) this.__s.shadowX = v; }
    get shadowOffsetY() { return this.__s.shadowY; }
    set shadowOffsetY(v) { v = +v; if (isFinite(v)) this.__s.shadowY = v; }
    get imageSmoothingEnabled() { return this.__s.smoothing; }
    set imageSmoothingEnabled(v) { this.__s.smoothing = !!v; }
    get imageSmoothingQuality() { return this.__s.smoothingQuality; }
    set imageSmoothingQuality(v) { if (["low", "medium", "high"].includes(v)) this.__s.smoothingQuality = v; }
    get filter() { return this.__s.filter; }
    set filter(v) { this.__s.filter = String(v); }

    // ---- transforms
    scale(x, y) { if (fin(x, y)) this.__s.m = mul(this.__s.m, [x, 0, 0, y, 0, 0]); }
    translate(x, y) { if (fin(x, y)) this.__s.m = mul(this.__s.m, [1, 0, 0, 1, +x, +y]); }
    rotate(a) { if (fin(a)) { const c = Math.cos(a), s = Math.sin(a); this.__s.m = mul(this.__s.m, [c, s, -s, c, 0, 0]); } }
    transform(a, b, c, d, e, f) { if (fin(a, b, c, d, e, f)) this.__s.m = mul(this.__s.m, [+a, +b, +c, +d, +e, +f]); }
    setTransform(a, b, c, d, e, f) {
      if (arguments.length === 0) return this.resetTransform();
      if (typeof a === "object" && a) { const t = a; return this.setTransform(t.a ?? t.m11 ?? 1, t.b ?? t.m12 ?? 0, t.c ?? t.m21 ?? 0, t.d ?? t.m22 ?? 1, t.e ?? t.m41 ?? 0, t.f ?? t.m42 ?? 0); }
      if (fin(a, b, c, d, e, f)) this.__s.m = [+a, +b, +c, +d, +e, +f];
    }
    resetTransform() { this.__s.m = [1, 0, 0, 1, 0, 0]; }
    getTransform() {
      const [a, b, c, d, e, f] = this.__s.m;
      if (typeof g.DOMMatrix === "function") return new g.DOMMatrix([a, b, c, d, e, f]);
      return { a, b, c, d, e, f, m11: a, m12: b, m21: c, m22: d, m41: e, m42: f, is2D: true };
    }

    // ---- paths
    beginPath() { this.__path = new PathData(); }

    __state(op) {
      const s = this.__s, sc = s.shadowColor;
      return new Float64Array([s.alpha, op ?? OPS[s.op], ...s.m, s.smoothing ? 1 : 0, s.shadowBlur, s.shadowX, s.shadowY, sc[0], sc[1], sc[2], sc[3]]);
    }
    __styleArgs(st) {
      if (st instanceof CanvasGradient) return [new Float64Array(st.__array()), 0, ""];
      if (st instanceof CanvasPattern) {
        const r = st.__rep;
        return [new Float64Array([3, r === "repeat" || r === "repeat-x" ? 1 : 0, r === "repeat" || r === "repeat-y" ? 1 : 0, ...(st.__m || [1, 0, 0, 1, 0, 0])]), st.__src[0], st.__src[1]];
      }
      return [new Float64Array([0, ...st]), 0, ""];
    }
    __pathArg(path) {
      const p = path instanceof Path2D ? path.__path.mapped(this.__s.m) : this.__path;
      return new Float64Array(p.c);
    }
    fill(a, b) {
      this.__check();
      const path = a instanceof Path2D ? a : null;
      const rule = (path ? b : a) === "evenodd" ? 1 : 0;
      const [spec, pk, pkey] = this.__styleArgs(this.__s.fill);
      __cvFill(this.__canvas.__id, this.__pathArg(path), spec, this.__state(), rule, pk, pkey);
    }
    stroke(path) {
      this.__check();
      const s = this.__s;
      const [spec, pk, pkey] = this.__styleArgs(s.stroke);
      const sk = new Float64Array([s.lineWidth, CAPS[s.cap], JOINS[s.join], s.miter, s.dashOffset, ...s.dash]);
      __cvStroke(this.__canvas.__id, this.__pathArg(path instanceof Path2D ? path : null), spec, this.__state(), sk, pk, pkey);
    }
    clip(a, b) {
      this.__check();
      const path = a instanceof Path2D ? a : null;
      const rule = (path ? b : a) === "evenodd" ? 1 : 0;
      __cvClip(this.__canvas.__id, this.__pathArg(path), rule);
    }
    isPointInPath(a, b, c, d) {
      let path = null, x, y, rule;
      if (a instanceof Path2D) [path, x, y, rule] = [a, b, c, d];
      else if (a !== null && typeof a === "object" && arguments.length > 2) throw new TypeError("isPointInPath: not a Path2D");
      else [x, y, rule] = [a, b, c];
      if (rule === undefined) rule = "nonzero";
      if (rule !== "nonzero" && rule !== "evenodd") throw new TypeError("isPointInPath: bad fill rule");
      x = +x; y = +y;
      if (!isFinite(x) || !isFinite(y)) return false;
      if (!invert(this.__s.m)) return false;
      const p = path ? path.__path.mapped(this.__s.m) : this.__path;
      return pointInPolys(p.polys(), x, y, rule === "evenodd");
    }
    isPointInStroke(a, b, c) {
      let path = null, x, y;
      if (a instanceof Path2D) [path, x, y] = [a, b, c];
      else if (a !== null && typeof a === "object") throw new TypeError("isPointInStroke: not a Path2D");
      else [x, y] = [a, b];
      x = +x; y = +y;
      if (!isFinite(x) || !isFinite(y)) return false;
      const inv = invert(this.__s.m);
      if (!inv) return false;
      // Measured in user space, where the line width and dashes are given.
      const p = path ? path.__path : this.__path.mapped(inv);
      const [ux, uy] = apply(inv, x, y);
      return pointOnStroke(p.polys(), ux, uy, this.__s);
    }
    drawFocusIfNeeded() {}
    scrollPathIntoView() {}

    // ---- rectangles
    __rectPath(x, y, w, h) { const p = new PathData(); p.rect(this.__s.m, x, y, w, h); return new Float64Array(p.c); }
    fillRect(x, y, w, h) {
      if (!fin(x, y, w, h) || !w || !h) return;
      this.__check();
      const [spec, pk, pkey] = this.__styleArgs(this.__s.fill);
      __cvFill(this.__canvas.__id, this.__rectPath(+x, +y, +w, +h), spec, this.__state(), 0, pk, pkey);
    }
    strokeRect(x, y, w, h) {
      if (!fin(x, y, w, h)) return;
      this.__check();
      const s = this.__s;
      const [spec, pk, pkey] = this.__styleArgs(s.stroke);
      const sk = new Float64Array([s.lineWidth, CAPS[s.cap], JOINS[s.join], s.miter, s.dashOffset, ...s.dash]);
      __cvStroke(this.__canvas.__id, this.__rectPath(+x, +y, +w, +h), spec, this.__state(), sk, pk, pkey);
    }
    clearRect(x, y, w, h) {
      if (!fin(x, y, w, h) || !w || !h) return;
      this.__check();
      const st = this.__state(26);
      st[0] = 1; st[15] = 0; // full strength, no shadow
      __cvFill(this.__canvas.__id, this.__rectPath(+x, +y, +w, +h), new Float64Array([0, 0, 0, 0, 1]), st, 0, 0, "");
    }

    // ---- gradients and patterns
    createLinearGradient(x0, y0, x1, y1) {
      if (!fin(x0, y0, x1, y1)) throw new TypeError("createLinearGradient: non-finite value");
      return new CanvasGradient([1, +x0, +y0, +x1, +y1]);
    }
    createRadialGradient(x0, y0, r0, x1, y1, r1) {
      if (!fin(x0, y0, r0, x1, y1, r1)) throw new TypeError("createRadialGradient: non-finite value");
      if (r0 < 0 || r1 < 0) throw new DOMException("createRadialGradient: negative radius", "IndexSizeError");
      return new CanvasGradient([2, +x0, +y0, +r0, +x1, +y1, +r1]);
    }
    createConicGradient(a, x, y) { return new CanvasGradient([4, +a, +x, +y]); }
    createPattern(img, rep) {
      rep = rep == null || rep === "" ? "repeat" : String(rep);
      if (!["repeat", "repeat-x", "repeat-y", "no-repeat"].includes(rep)) throw new DOMException("createPattern: bad repetition", "SyntaxError");
      if (img && img.nodeType === 1 && img.localName === "canvas" && (canvasDim(img, "width", 300) === 0 || canvasDim(img, "height", 150) === 0)) {
        throw new DOMException("createPattern: the canvas has no pixels", "InvalidStateError");
      }
      let src = sourceOf(img);
      if (!src) return null;
      // A canvas pattern is a copy of the canvas as it is now.
      if (src[0] === 1) {
        const copy = hiddenCanvas();
        copy.__offscreenSize = [src[2], src[3]];
        getContext.call(copy, "2d").drawImage(img, 0, 0);
        src = [1, String(copy.__id), src[2], src[3]];
      }
      return new CanvasPattern(src, rep);
    }

    // ---- text
    __fontArgs() { const f = this.__s.font; return [new Float64Array([f.size, f.bold ? 1 : 0, f.italic ? 1 : 0]), f.family]; }
    __text(text, x, y, maxWidth, stroke) {
      if (!fin(x, y)) return;
      if (maxWidth !== undefined && !(+maxWidth > 0)) return;
      this.__check();
      const s = this.__s;
      let align = ALIGNS[s.align];
      const rtl = s.direction === "rtl" || (s.direction === "inherit" && (this.__canvas.getAttribute && this.__canvas.getAttribute("dir")) === "rtl");
      if (rtl && s.align === "start") align = 2;
      if (rtl && s.align === "end") align = 0;
      const [font, family] = this.__fontArgs();
      const [spec] = this.__styleArgs(stroke ? s.stroke : s.fill);
      const pos = new Float64Array([+x, +y, maxWidth === undefined ? NaN : +maxWidth, align, BASELINES[s.baseline], stroke ? 1 : 0, s.lineWidth]);
      __cvText(this.__canvas.__id, String(text).replace(/[\t\n\f\r]/g, " "), pos, font, family, spec, this.__state(), 0);
    }
    fillText(text, x, y, maxWidth) { this.__text(text, x, y, maxWidth, false); }
    strokeText(text, x, y, maxWidth) { this.__text(text, x, y, maxWidth, true); }
    measureText(text) {
      const [font, family] = this.__fontArgs();
      const [w, asc, desc] = __cvMeasure(String(text), font, family);
      const t = new TextMetrics();
      const align = ALIGNS[this.__s.align];
      const left = align === 1 ? w / 2 : align === 2 ? w : 0;
      Object.assign(t, {
        width: w, actualBoundingBoxLeft: left, actualBoundingBoxRight: w - left,
        actualBoundingBoxAscent: asc, actualBoundingBoxDescent: desc, fontBoundingBoxAscent: asc, fontBoundingBoxDescent: desc,
        emHeightAscent: asc, emHeightDescent: desc, hangingBaseline: asc * 0.8, alphabeticBaseline: 0, ideographicBaseline: -desc,
      });
      return t;
    }

    // ---- images
    drawImage(img, ...a) {
      if (![2, 4, 8].includes(a.length)) throw new TypeError("drawImage: wrong number of arguments");
      if (!a.every((v) => isFinite(+v))) return;
      this.__check();
      const src = sourceOf(img);
      if (!src) return;
      const [kind, key, iw, ih] = src;
      let r;
      if (a.length === 2) r = [0, 0, iw, ih, +a[0], +a[1], iw, ih];
      else if (a.length === 4) r = [0, 0, iw, ih, +a[0], +a[1], +a[2], +a[3]];
      else r = a.map(Number);
      __cvImage(this.__canvas.__id, kind, key, new Float64Array(r), this.__state());
    }
    createImageData(a, b, settings) {
      if (!(this instanceof CanvasRenderingContext2D)) throw new TypeError("createImageData: illegal invocation");
      if (a instanceof ImageData) return new ImageData(a.width, a.height);
      if (arguments.length < 2 || a === null || typeof a === "object") throw new TypeError("createImageData: needs an ImageData or a width and height");
      if (!fin(a, b)) throw new TypeError("createImageData: non-finite value");
      const w = Math.abs(Math.trunc(+a)), h = Math.abs(Math.trunc(+b));
      if (!w || !h) throw new DOMException("createImageData: zero size", "IndexSizeError");
      return new ImageData(w, h, settings);
    }
    getImageData(x, y, w, h) {
      if (!(this instanceof CanvasRenderingContext2D)) throw new TypeError("getImageData: illegal invocation");
      if (!fin(x, y, w, h)) throw new TypeError("getImageData: non-finite value");
      [x, y, w, h] = [x, y, w, h].map((v) => Math.trunc(+v));
      if (!w || !h) throw new DOMException("getImageData: zero size", "IndexSizeError");
      this.__check();
      if (w < 0) { x += w; w = -w; }
      if (h < 0) { y += h; h = -h; }
      if (w * h * 4 > 0x7fffffff) throw new RangeError("getImageData: too large");
      const data = __cvGetData(this.__canvas.__id, x, y, w, h);
      return new ImageData(data, w, h);
    }
    putImageData(img, dx, dy, ...dirty) {
      if (!(img instanceof ImageData)) throw new TypeError("putImageData: not ImageData");
      if (!fin(dx, dy, ...dirty)) throw new TypeError("putImageData: non-finite value");
      this.__check();
      __cvPutData(this.__canvas.__id, img.data, img.width, img.height, Math.trunc(+dx), Math.trunc(+dy), dirty.length === 4 ? new Float64Array(dirty.map((v) => Math.trunc(+v))) : null);
    }
  }
  Object.assign(CanvasRenderingContext2D.prototype, pathMethods);
  // The path methods mutate the current path through the current transform.

  function styleIn(v, canvas) {
    if (v instanceof CanvasGradient || v instanceof CanvasPattern) return v;
    if (typeof v === "string" || (v && typeof v === "object" && typeof v.toString === "function")) {
      const str = String(v).trim();
      // currentColor is the canvas element's color now (black when it isn't in a document).
      if (/^currentcolor$/i.test(str)) {
        try {
          if (canvas && canvas.isConnected && typeof getComputedStyle === "function") return parseColor(getComputedStyle(canvas).color) || [0, 0, 0, 1];
        } catch (e) {}
        return [0, 0, 0, 1];
      }
      return parseColor(str);
    }
    return null;
  }
  function styleOut(v) { return Array.isArray(v) ? serializeColor(v) : v; }

  function canvasDim(el, name, dflt) {
    if (el.__offscreenSize) return el.__offscreenSize[name === "width" ? 0 : 1];
    const v = el.getAttribute(name);
    if (v == null) return dflt;
    const m = /^\s*(\d+)/.exec(v);
    return m ? parseInt(m[1], 10) : dflt;
  }

  // ---- the <canvas> element
  const contexts = new Map();
  function getContext(type, attrs) {
    type = String(type);
    if (type === "webgl" || type === "experimental-webgl" || type === "webgl2") {
      if (contexts.has(this.__id) || !g.__webglGetContext) return null;
      return g.__webglGetContext(this, type === "webgl2" ? "webgl2" : "webgl", attrs);
    }
    if (type !== "2d") return null;
    if (g.__webglHas && g.__webglHas(this.__id)) return null;
    let c = contexts.get(this.__id);
    if (!c) { c = new CanvasRenderingContext2D(this); contexts.set(this.__id, c); }
    return c;
  }
  function toDataURL(type, q) {
    if (g.__glFlushCanvas) g.__glFlushCanvas(this.__id);
    const c = contexts.get(this.__id);
    if (!c) {
      // A canvas never drawn on is still a blank image of its size.
      getContext.call(this, "2d");
    }
    if (canvasDim(this, "width", 300) === 0 || canvasDim(this, "height", 150) === 0) return "data:,";
    return __cvDataURL(this.__id, String(type || "image/png"), q);
  }
  function toBlob(cb, type, q) {
    if (typeof cb !== "function") throw new TypeError("toBlob: callback required");
    const url = toDataURL.call(this, type, q);
    setTimeout(() => {
      if (url === "data:,") return cb(null);
      const bin = atob(url.slice(url.indexOf(",") + 1));
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      cb(new Blob([bytes], { type: "image/png" }));
    }, 0);
  }
  const isCanvas = (el) => el && el.localName === "canvas";
  const proto = g.Element.prototype;
  // Canvas-only members on the shared element prototype: other elements keep whatever
  // they had before (or nothing).
  function canvasProp(name, get, set) {
    const prev = Object.getOwnPropertyDescriptor(proto, name) || Object.getOwnPropertyDescriptor(g.HTMLElement.prototype, name);
    Object.defineProperty(proto, name, {
      configurable: true,
      get() {
        if (isCanvas(this)) return get.call(this);
        if (prev && prev.get) return prev.get.call(this);
        return prev ? prev.value : undefined;
      },
      set(v) {
        if (isCanvas(this) && set) return set.call(this, v);
        if (prev && prev.set) return prev.set.call(this, v);
        Object.defineProperty(this, name, { value: v, writable: true, configurable: true, enumerable: true });
      },
    });
  }
  canvasProp("getContext", function () { return getContext; });
  canvasProp("toDataURL", function () { return toDataURL; });
  canvasProp("toBlob", function () { return toBlob; });
  canvasProp("transferControlToOffscreen", function () { return () => { throw new DOMException("not supported", "NotSupportedError"); }; });
  for (const [name, dflt] of [["width", 300], ["height", 150]]) {
    canvasProp(name, function () { return canvasDim(this, name, dflt); }, function (v) {
      v = Math.max(0, Math.floor(+v) || 0);
      this.setAttribute(name, String(v));
      const c = contexts.get(this.__id);
      if (c) c.__reset(); // setting the size clears the canvas, even to the same value
      if (g.__glResizeCanvas) g.__glResizeCanvas(this);
    });
  }

  // OffscreenCanvas: a canvas element that isn't in the document.
  class OffscreenCanvas {
    constructor(w, h) {
      this.__offscreen = hiddenCanvas();
      this.__offscreen.__offscreenSize = [Math.max(0, w | 0), Math.max(0, h | 0)];
      this.__offscreen.__offscreenOwner = this;
    }
    get width() { return this.__offscreen.__offscreenSize[0]; }
    set width(v) { this.__offscreen.__offscreenSize[0] = Math.max(0, v | 0); const c = contexts.get(this.__offscreen.__id); if (c) c.__reset(); if (g.__glResizeCanvas) g.__glResizeCanvas(this.__offscreen); }
    get height() { return this.__offscreen.__offscreenSize[1]; }
    set height(v) { this.__offscreen.__offscreenSize[1] = Math.max(0, v | 0); const c = contexts.get(this.__offscreen.__id); if (c) c.__reset(); if (g.__glResizeCanvas) g.__glResizeCanvas(this.__offscreen); }
    getContext(type, attrs) { return getContext.call(this.__offscreen, type, attrs); }
    convertToBlob(opts) {
      return new Promise((res) => toBlob.call(this.__offscreen, res, opts && opts.type));
    }
    transferToImageBitmap() {
      const b = new ImageBitmap(this.width, this.height);
      b.__ctx.drawImage(this.__offscreen, 0, 0);
      return b;
    }
  }

  // ImageBitmap: a copy of an image's pixels, held in a hidden canvas.
  class ImageBitmap {
    constructor(w, h) {
      this.__bitmap = hiddenCanvas();
      this.__bitmap.__offscreenSize = [w, h];
      this.__ctx = getContext.call(this.__bitmap, "2d");
    }
    get width() { return this.__bitmap.__offscreenSize[0]; }
    get height() { return this.__bitmap.__offscreenSize[1]; }
    close() {}
  }
  // createImageBitmap(source[, sx, sy, sw, sh][, options]): options imageOrientation
  // ("flipY"), premultiplyAlpha ("none" / "premultiply", kept for WebGL uploads) and
  // resizeWidth/resizeHeight are applied when the copy is made.
  function createImageBitmap(img, ...a) {
    return new Promise((resolve, reject) => {
      const opts = (a.length === 1 ? a[0] : a[4]) || {};
      const rect = a.length >= 4 ? a.slice(0, 4).map(Number) : null;
      if (rect && (rect[2] === 0 || rect[3] === 0)) return reject(new RangeError("createImageBitmap: empty source rectangle"));
      // Copy `from` (anything drawImage takes) into a new bitmap.
      // straight: {data, w, h} unpremultiplied pixels when the source has them (ImageData, or
      // a bitmap made from one), kept so WebGL uploads don't lose color under zero alpha.
      const make = (from, w, h, straight) => {
        let [sx, sy, sw, sh] = rect || [0, 0, w, h];
        if (sw < 0) { sx += sw; sw = -sw; }
        if (sh < 0) { sy += sh; sh = -sh; }
        const dw = opts.resizeWidth > 0 ? opts.resizeWidth : opts.resizeHeight > 0 ? Math.round(sw * opts.resizeHeight / sh) : sw;
        const dh = opts.resizeHeight > 0 ? opts.resizeHeight : opts.resizeWidth > 0 ? Math.round(sh * opts.resizeWidth / sw) : sh;
        const b = new ImageBitmap(dw, dh);
        if (opts.imageOrientation === "flipY") b.__ctx.setTransform(1, 0, 0, -1, 0, dh);
        b.__ctx.drawImage(from, sx, sy, sw, sh, 0, 0, dw, dh);
        b.__ctx.setTransform(1, 0, 0, 1, 0, 0);
        b.__bitmapPremultiply = opts.premultiplyAlpha === "premultiply";
        if (straight && dw === sw && dh === sh && sx >= 0 && sy >= 0 && sx + sw <= w && sy + sh <= h) {
          const out = new Uint8ClampedArray(dw * dh * 4);
          for (let y = 0; y < dh; y++) {
            const from = ((sy + (opts.imageOrientation === "flipY" ? dh - 1 - y : y)) * w + sx) * 4;
            out.set(straight.data.subarray(from, from + dw * 4), y * dw * 4);
          }
          b.__straight = { data: out, w: dw, h: dh };
        }
        return b;
      };
      try {
        if (img instanceof ImageData) {
          const c = hiddenCanvas();
          c.__offscreenSize = [img.width, img.height];
          getContext.call(c, "2d").putImageData(img, 0, 0);
          return resolve(make(c, img.width, img.height, { data: img.data }));
        }
        if (typeof Blob === "function" && img instanceof Blob) {
          return img.arrayBuffer().then((buf) => {
            const c = hiddenCanvas();
            c.__offscreenSize = [0, 0];
            const cx = getContext.call(c, "2d");
            const size = __cvDecodeInto(c.__id, new Uint8Array(buf));
            if (!size) throw new DOMException("createImageBitmap: the image can't be decoded", "InvalidStateError");
            c.__offscreenSize = [size[0], size[1]];
            cx.__size = [size[0], size[1]];
            resolve(make(c, size[0], size[1]));
          }).catch(reject);
        }
        const src = sourceOf(img);
        if (!src) return reject(new DOMException("createImageBitmap: image not loaded", "InvalidStateError"));
        resolve(make(img, src[2], src[3], img.__straight));
      } catch (e) {
        reject(e);
      }
    });
  }

  Object.assign(g, { CanvasRenderingContext2D, Path2D, CanvasGradient, CanvasPattern, ImageData, TextMetrics, OffscreenCanvas, ImageBitmap, createImageBitmap });
  g.OffscreenCanvasRenderingContext2D = CanvasRenderingContext2D;
})();
