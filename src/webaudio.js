// Web Audio: the parts pages use to make and measure sound — AudioContext and
// OfflineAudioContext, oscillators, gain, filters, the compressor, buffers and the analyser.
// The graph is rendered here in 128-frame blocks, one channel (stereo outputs carry the same
// samples on each channel). An OfflineAudioContext renders when asked; an AudioContext renders
// lazily up to its clock whenever an analyser is read. Nothing is played aloud yet.
//
// Fingerprinting protection: everything a page can read back (rendered buffers, analyser data)
// gets a little per-site noise from __fpSeed(), the same seed canvas readouts use — the same
// values every time on one site, different values on another site or in another session.
(function (g) {
  "use strict";
  const BLOCK = 128;

  // ---------------------------------------------------------------- readout noise

  const hash = (seed, i) => {
    let x = (seed ^ Math.imul(i + 1, 0x9e3779b9)) >>> 0;
    x = Math.imul(x ^ (x >>> 16), 0x85ebca6b) >>> 0;
    x = Math.imul(x ^ (x >>> 13), 0xc2b2ae35) >>> 0;
    return (x ^ (x >>> 16)) >>> 0;
  };
  // Taken now, so a page can't swap it out to turn the noise off.
  const fpSeed = g.__fpSeed;
  const seed = () => (typeof fpSeed === "function" ? fpSeed() >>> 0 : 0);
  // A hash of the values themselves: the noise pattern depends on the sound, so a page can't
  // learn it from a known signal and divide it out of another.
  function contentHash(arr) {
    let h = 0x811c9dc5;
    const step = Math.max(1, Math.floor(arr.length / 4096));
    for (let i = 0; i < arr.length; i += step) h = Math.imul(h ^ (Math.round(arr[i] * 1e5) | 0), 0x01000193) >>> 0;
    return h;
  }
  // Samples: about one in eight non-silent values is scaled by 1 ± a few millionths.
  function noiseFloats(arr, salt) {
    const s = seed();
    if (!s) return arr;
    const k = (s ^ salt ^ contentHash(arr)) >>> 0;
    for (let i = 0; i < arr.length; i++) {
      const v = arr[i];
      if (v === 0 || !isFinite(v)) continue;
      const h = hash(k, i);
      if ((h & 7) !== 0) continue;
      const d = (1 + ((h >>> 3) & 7)) * 1e-6;
      arr[i] = v * (h & 0x400 ? 1 + d : 1 - d);
    }
    return arr;
  }
  // Bytes: about one value in a hundred moves by ±1, like canvas pixels.
  function noiseBytes(arr, salt) {
    const s = seed();
    if (!s) return arr;
    const k = (s ^ salt ^ contentHash(arr)) >>> 0;
    for (let i = 0; i < arr.length; i++) {
      const h = hash(k, i);
      if (h % 97 !== 0) continue;
      arr[i] = h & 0x100 ? Math.min(255, arr[i] + 1) : Math.max(0, arr[i] - 1);
    }
    return arr;
  }

  const domErr = (msg, name) => (typeof g.DOMException === "function" ? new g.DOMException(msg, name) : new Error(msg));

  // ---------------------------------------------------------------- AudioParam

  class AudioParam {
    constructor(ctx, value, min = -3.4028234663852886e38, max = 3.4028234663852886e38) {
      this._ctx = ctx;
      this._v = value;
      this.defaultValue = value;
      this.minValue = min;
      this.maxValue = max;
      this.automationRate = "a-rate";
      this._ev = [];
      this._inputs = [];
    }
    get value() {
      return this._ev.length ? this._at(this._ctx.currentTime) : this._v;
    }
    set value(v) {
      this._v = Number(v);
      this._ev = this._ev.filter((e) => e.t > this._ctx.currentTime);
    }
    _add(e) {
      if (!isFinite(e.t) || e.t < 0) throw new RangeError("Invalid time");
      this._ev.push(e);
      this._ev.sort((a, b) => a.t - b.t);
      return this;
    }
    setValueAtTime(v, t) { return this._add({ k: "set", v: Number(v), t: Number(t) }); }
    linearRampToValueAtTime(v, t) { return this._add({ k: "lin", v: Number(v), t: Number(t) }); }
    exponentialRampToValueAtTime(v, t) {
      if (Number(v) === 0) throw new RangeError("The value can't be zero");
      return this._add({ k: "exp", v: Number(v), t: Number(t) });
    }
    setTargetAtTime(v, t, c) { return this._add({ k: "target", v: Number(v), t: Number(t), c: Math.max(Number(c), 1e-9) }); }
    setValueCurveAtTime(curve, t, d) { return this._add({ k: "curve", curve: Float32Array.from(curve), t: Number(t), d: Number(d) }); }
    cancelScheduledValues(t) { this._ev = this._ev.filter((e) => e.t < t); return this; }
    cancelAndHoldAtTime(t) { const v = this._at(t); this.cancelScheduledValues(t); return this.setValueAtTime(v, t); }
    // The value at time t from the scheduled events.
    _at(t) {
      let v = this._v, prevT = 0;
      for (let i = 0; i < this._ev.length; i++) {
        const e = this._ev[i];
        if (e.k === "lin" || e.k === "exp") {
          if (t >= e.t) { v = e.v; prevT = e.t; continue; }
          const f = (t - prevT) / (e.t - prevT || 1);
          if (e.k === "lin") return v + (e.v - v) * f;
          return v !== 0 && v * e.v > 0 ? v * Math.pow(e.v / v, f) : v;
        }
        if (e.t > t) break;
        if (e.k === "set") v = e.v;
        else if (e.k === "curve") {
          const c = e.curve, f = Math.min(1, (t - e.t) / (e.d || 1)) * (c.length - 1);
          const i0 = Math.floor(f), i1 = Math.min(c.length - 1, i0 + 1);
          v = c[i0] + (c[i1] - c[i0]) * (f - i0);
        } else if (e.k === "target") {
          const next = this._ev[i + 1];
          const end = next && next.t <= t ? next.t : t;
          v = e.v + (v - e.v) * Math.exp(-(end - e.t) / e.c);
        }
        prevT = e.t;
      }
      return v;
    }
    // Per-frame values for the block starting at frame f0.
    _block(f0) {
      const out = new Float32Array(BLOCK);
      const sr = this._ctx.sampleRate;
      if (!this._ev.length) out.fill(this._v);
      else for (let i = 0; i < BLOCK; i++) out[i] = this._at((f0 + i) / sr);
      for (const n of this._inputs) {
        const x = n._pull(f0);
        for (let i = 0; i < BLOCK; i++) out[i] += x[i];
      }
      for (let i = 0; i < BLOCK; i++) out[i] = Math.min(this.maxValue, Math.max(this.minValue, out[i]));
      return out;
    }
  }

  // ---------------------------------------------------------------- nodes

  class AudioNode extends g.EventTarget {
    constructor(ctx, ins = 1, outs = 1) {
      super();
      if (!(ctx instanceof BaseAudioContext)) throw new TypeError("Failed to construct an audio node: the first argument must be an audio context");
      this._context = ctx;
      this._inputs = [];
      this._outs = new Set();
      this._cacheF = -1;
      this._cache = null;
      this.numberOfInputs = ins;
      this.numberOfOutputs = outs;
      this.channelCount = 2;
      this.channelCountMode = "max";
      this.channelInterpretation = "speakers";
    }
    get context() { return this._context; }
    connect(dest) {
      if (dest instanceof AudioParam) {
        if (!dest._inputs.includes(this)) dest._inputs.push(this);
        this._outs.add(dest);
        return undefined;
      }
      if (!(dest instanceof AudioNode)) throw new TypeError("Failed to execute 'connect': not an AudioNode or AudioParam");
      if (dest._context !== this._context) throw domErr("The destination belongs to a different audio context", "InvalidAccessError");
      if (!dest._inputs.includes(this)) dest._inputs.push(this);
      this._outs.add(dest);
      return dest;
    }
    disconnect(dest) {
      const targets = dest ? [dest] : [...this._outs];
      for (const d of targets) {
        d._inputs = d._inputs.filter((n) => n !== this);
        this._outs.delete(d);
      }
    }
    // The sum of everything connected to this node, for the block at frame f0.
    _in(f0) {
      const out = new Float32Array(BLOCK);
      for (const n of this._inputs) {
        const x = n._pull(f0);
        for (let i = 0; i < BLOCK; i++) out[i] += x[i];
      }
      return out;
    }
    // This node's output for the block at frame f0 (each block is worked out once).
    _pull(f0) {
      if (this._cacheF === f0) return this._cache;
      // A cycle without a delay: silence rather than endless recursion.
      if (this._busy) return new Float32Array(BLOCK);
      this._busy = true;
      try { this._cache = this._process(f0); } finally { this._busy = false; }
      this._cacheF = f0;
      return this._cache;
    }
    _process(f0) { return this._in(f0); }
  }

  class AudioDestinationNode extends AudioNode {
    constructor(ctx, channels) {
      super(ctx, 1, 0);
      this.maxChannelCount = channels;
      this.channelCount = channels;
    }
  }

  // Start/stop and the ended event for sources.
  class AudioScheduledSourceNode extends AudioNode {
    constructor(ctx) {
      super(ctx, 0, 1);
      this._start = Infinity;
      this._stop = Infinity;
      this._ended = false;
    }
    start(when = 0) {
      if (this._start !== Infinity) throw domErr("start() may only be called once", "InvalidStateError");
      this._start = Math.max(0, Number(when) || 0);
    }
    stop(when = 0) {
      if (this._start === Infinity) throw domErr("stop() before start()", "InvalidStateError");
      this._stop = Math.max(0, Number(when) || 0);
    }
    _active(f0) {
      const sr = this._context.sampleRate;
      const a = Math.ceil(this._start * sr), b = Math.ceil(this._stop * sr);
      if (f0 >= b && !this._ended && this._start !== Infinity) this._end();
      return [Math.max(0, a - f0), Math.min(BLOCK, b - f0)];
    }
    _end() {
      this._ended = true;
      this._context._later(() => this.dispatchEvent(new g.Event("ended")));
    }
  }

  class OscillatorNode extends AudioScheduledSourceNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.frequency = new AudioParam(ctx, o.frequency ?? 440, -ctx.sampleRate / 2, ctx.sampleRate / 2);
      this.detune = new AudioParam(ctx, o.detune ?? 0);
      this._type = "sine";
      this._phase = 0;
      this._wave = null;
      if (o.periodicWave) this.setPeriodicWave(o.periodicWave);
      else if (o.type) this.type = o.type;
    }
    get type() { return this._type; }
    set type(t) {
      if (!["sine", "square", "sawtooth", "triangle"].includes(t)) {
        if (t === "custom") throw domErr("Use setPeriodicWave() for a custom wave", "InvalidStateError");
        return;
      }
      this._type = t;
      this._wave = null;
    }
    setPeriodicWave(w) {
      this._wave = w;
      this._type = "custom";
    }
    _process(f0) {
      const out = new Float32Array(BLOCK);
      const [a, b] = this._active(f0);
      if (a >= b) return out;
      const fr = this.frequency._block(f0), dt = this.detune._block(f0);
      const sr = this._context.sampleRate;
      for (let i = a; i < b; i++) {
        const f = fr[i] * Math.pow(2, dt[i] / 1200);
        const p = this._phase;
        let v;
        switch (this._type) {
          case "sine": v = Math.sin(2 * Math.PI * p); break;
          case "square": v = p < 0.5 ? 1 : -1; break;
          case "sawtooth": v = p < 0.5 ? 2 * p : 2 * p - 2; break;
          case "triangle": v = p < 0.25 ? 4 * p : p < 0.75 ? 2 - 4 * p : 4 * p - 4; break;
          default: v = this._wave._at(p);
        }
        out[i] = v;
        this._phase = (p + f / sr) % 1;
        if (this._phase < 0) this._phase += 1;
      }
      return out;
    }
  }

  class PeriodicWave {
    constructor(ctx, o = {}) {
      const real = Float32Array.from(o.real || [0, 0]);
      const imag = Float32Array.from(o.imag || [0, 1]);
      if (real.length !== imag.length) throw domErr("real and imag must be the same length", "IndexSizeError");
      // One period as a table, normalized to a peak of 1 unless asked not to.
      const N = 2048, t = new Float32Array(N);
      for (let i = 0; i < N; i++) {
        let v = 0;
        for (let k = 1; k < real.length; k++) {
          const w = (2 * Math.PI * k * i) / N;
          v += real[k] * Math.cos(w) + imag[k] * Math.sin(w);
        }
        t[i] = v;
      }
      if (!o.disableNormalization) {
        let m = 0;
        for (const v of t) m = Math.max(m, Math.abs(v));
        if (m > 0) for (let i = 0; i < N; i++) t[i] /= m;
      }
      this._t = t;
    }
    _at(p) {
      const t = this._t, f = p * t.length, i = Math.floor(f) % t.length;
      return t[i] + (t[(i + 1) % t.length] - t[i]) * (f - Math.floor(f));
    }
  }

  class ConstantSourceNode extends AudioScheduledSourceNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.offset = new AudioParam(ctx, o.offset ?? 1);
    }
    _process(f0) {
      const out = new Float32Array(BLOCK);
      const [a, b] = this._active(f0);
      if (a >= b) return out;
      const v = this.offset._block(f0);
      for (let i = a; i < b; i++) out[i] = v[i];
      return out;
    }
  }

  class AudioBufferSourceNode extends AudioScheduledSourceNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.buffer = o.buffer ?? null;
      this.loop = !!o.loop;
      this.loopStart = o.loopStart ?? 0;
      this.loopEnd = o.loopEnd ?? 0;
      this.playbackRate = new AudioParam(ctx, o.playbackRate ?? 1);
      this.detune = new AudioParam(ctx, o.detune ?? 0);
      this._pos = 0;
    }
    start(when = 0, offset = 0, duration) {
      super.start(when);
      this._pos = Math.max(0, Number(offset) || 0) * (this.buffer ? this.buffer.sampleRate : this._context.sampleRate);
      if (duration !== undefined) this._stop = this._start + Math.max(0, Number(duration));
    }
    _process(f0) {
      const out = new Float32Array(BLOCK);
      const [a, b] = this._active(f0);
      const buf = this.buffer;
      if (a >= b || !buf) return out;
      const rate = this.playbackRate._block(f0), dt = this.detune._block(f0);
      const step0 = buf.sampleRate / this._context.sampleRate;
      const chans = buf._ch;
      const ls = this.loopStart * buf.sampleRate, le = this.loopEnd > 0 ? Math.min(buf.length, this.loopEnd * buf.sampleRate) : buf.length;
      for (let i = a; i < b; i++) {
        if (this.loop && this._pos >= le && le > ls) this._pos = ls + ((this._pos - ls) % (le - ls));
        if (this._pos >= buf.length || this._pos < 0) {
          if (!this._ended) this._end();
          break;
        }
        const j = Math.floor(this._pos), fr = this._pos - j;
        let v = 0;
        for (const c of chans) v += c[j] + ((j + 1 < c.length ? c[j + 1] : 0) - c[j]) * fr;
        out[i] = v / chans.length;
        this._pos += step0 * rate[i] * Math.pow(2, dt[i] / 1200);
      }
      return out;
    }
  }

  class GainNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.gain = new AudioParam(ctx, o.gain ?? 1);
    }
    _process(f0) {
      const x = this._in(f0), gn = this.gain._block(f0);
      for (let i = 0; i < BLOCK; i++) x[i] *= gn[i];
      return x;
    }
  }

  class StereoPannerNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.pan = new AudioParam(ctx, o.pan ?? 0, -1, 1);
    }
  }

  class DelayNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this._max = o.maxDelayTime ?? 1;
      this.delayTime = new AudioParam(ctx, o.delayTime ?? 0, 0, this._max);
      this._ring = new Float32Array(Math.ceil(this._max * ctx.sampleRate) + BLOCK + 1);
      this._w = 0;
    }
    _process(f0) {
      const x = this._in(f0), d = this.delayTime._block(f0), r = this._ring, n = r.length, sr = this._context.sampleRate;
      const out = new Float32Array(BLOCK);
      for (let i = 0; i < BLOCK; i++) {
        r[this._w] = x[i];
        const back = d[i] * sr, pos = (this._w - back + n * 2) % n, j = Math.floor(pos), fr = pos - j;
        out[i] = r[j] + (r[(j + 1) % n] - r[j]) * fr;
        this._w = (this._w + 1) % n;
      }
      return out;
    }
  }

  class BiquadFilterNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.type = o.type ?? "lowpass";
      this.frequency = new AudioParam(ctx, o.frequency ?? 350, 0, ctx.sampleRate / 2);
      this.detune = new AudioParam(ctx, o.detune ?? 0);
      this.Q = new AudioParam(ctx, o.Q ?? 1);
      this.gain = new AudioParam(ctx, o.gain ?? 0);
      this._z = [0, 0, 0, 0];
    }
    // RBJ cookbook coefficients, normalized: [b0, b1, b2, a1, a2].
    _coef(f, q, gdb, detune) {
      const sr = this._context.sampleRate;
      const fr = Math.min(f * Math.pow(2, detune / 1200), sr / 2);
      const w = (2 * Math.PI * fr) / sr, cw = Math.cos(w), sw = Math.sin(w);
      const A = Math.pow(10, gdb / 40);
      const Qv = Math.max(q, 1e-4);
      let alpha = sw / (2 * Qv);
      let b0, b1, b2, a0, a1, a2;
      switch (this.type) {
        case "highpass": b0 = (1 + cw) / 2; b1 = -(1 + cw); b2 = b0; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha; break;
        case "bandpass": b0 = alpha; b1 = 0; b2 = -alpha; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha; break;
        case "notch": b0 = 1; b1 = -2 * cw; b2 = 1; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha; break;
        case "allpass": b0 = 1 - alpha; b1 = -2 * cw; b2 = 1 + alpha; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha; break;
        case "peaking": b0 = 1 + alpha * A; b1 = -2 * cw; b2 = 1 - alpha * A; a0 = 1 + alpha / A; a1 = -2 * cw; a2 = 1 - alpha / A; break;
        case "lowshelf":
        case "highshelf": {
          alpha = (sw / 2) * Math.sqrt(2);
          const s = 2 * Math.sqrt(A) * alpha, sign = this.type === "lowshelf" ? 1 : -1;
          b0 = A * ((A + 1) - sign * (A - 1) * cw + s);
          b1 = sign * 2 * A * ((A - 1) - sign * (A + 1) * cw);
          b2 = A * ((A + 1) - sign * (A - 1) * cw - s);
          a0 = (A + 1) + sign * (A - 1) * cw + s;
          a1 = -sign * 2 * ((A - 1) + sign * (A + 1) * cw);
          a2 = (A + 1) + sign * (A - 1) * cw - s;
          break;
        }
        default: // lowpass (Q in dB, as Web Audio defines it)
          alpha = sw / (2 * Math.pow(10, q / 20));
          b0 = (1 - cw) / 2; b1 = 1 - cw; b2 = b0; a0 = 1 + alpha; a1 = -2 * cw; a2 = 1 - alpha;
      }
      return [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
    }
    _process(f0) {
      const x = this._in(f0), out = new Float32Array(BLOCK);
      const [b0, b1, b2, a1, a2] = this._coef(this.frequency._block(f0)[0], this.Q._block(f0)[0], this.gain._block(f0)[0], this.detune._block(f0)[0]);
      let [x1, x2, y1, y2] = this._z;
      for (let i = 0; i < BLOCK; i++) {
        const y = b0 * x[i] + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1; x1 = x[i]; y2 = y1; y1 = y;
        out[i] = y;
      }
      this._z = [x1, x2, y1, y2];
      return out;
    }
    getFrequencyResponse(freqs, mag, phase) {
      const [b0, b1, b2, a1, a2] = this._coef(this.frequency.value, this.Q.value, this.gain.value, this.detune.value);
      const sr = this._context.sampleRate;
      for (let i = 0; i < freqs.length; i++) {
        const f = freqs[i];
        if (!(f >= 0 && f <= sr / 2)) { mag[i] = NaN; phase[i] = NaN; continue; }
        const w = (2 * Math.PI * f) / sr;
        const zr1 = Math.cos(-w), zi1 = Math.sin(-w), zr2 = Math.cos(-2 * w), zi2 = Math.sin(-2 * w);
        const nr = b0 + b1 * zr1 + b2 * zr2, ni = b1 * zi1 + b2 * zi2;
        const dr = 1 + a1 * zr1 + a2 * zr2, di = a1 * zi1 + a2 * zi2;
        const dd = dr * dr + di * di;
        const hr = (nr * dr + ni * di) / dd, hi = (ni * dr - nr * di) / dd;
        mag[i] = Math.hypot(hr, hi);
        phase[i] = Math.atan2(hi, hr);
      }
    }
  }

  class WaveShaperNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.curve = o.curve ? Float32Array.from(o.curve) : null;
      this.oversample = o.oversample ?? "none";
    }
    _process(f0) {
      const x = this._in(f0), c = this.curve;
      if (!c || c.length < 2) return x;
      for (let i = 0; i < BLOCK; i++) {
        const v = ((Math.max(-1, Math.min(1, x[i])) + 1) / 2) * (c.length - 1), j = Math.floor(v);
        x[i] = j >= c.length - 1 ? c[c.length - 1] : c[j] + (c[j + 1] - c[j]) * (v - j);
      }
      return x;
    }
  }

  // A feed-forward compressor with a soft knee, attack/release smoothing and makeup gain.
  class DynamicsCompressorNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this.threshold = new AudioParam(ctx, o.threshold ?? -24, -100, 0);
      this.knee = new AudioParam(ctx, o.knee ?? 30, 0, 40);
      this.ratio = new AudioParam(ctx, o.ratio ?? 12, 1, 20);
      this.attack = new AudioParam(ctx, o.attack ?? 0.003, 0, 1);
      this.release = new AudioParam(ctx, o.release ?? 0.25, 0, 1);
      this._env = 0;
      this._red = 0;
    }
    get reduction() { return this._red; }
    _curve(db, T, K, R) {
      if (db < T - K / 2) return db;
      if (K > 0 && db <= T + K / 2) return db + ((1 / R - 1) * (db - T + K / 2) ** 2) / (2 * K);
      return T + (db - T) / R;
    }
    _process(f0) {
      const x = this._in(f0), out = new Float32Array(BLOCK), sr = this._context.sampleRate;
      const T = this.threshold._block(f0)[0], K = this.knee._block(f0)[0], R = this.ratio._block(f0)[0];
      const at = Math.exp(-1 / (Math.max(this.attack._block(f0)[0], 1e-4) * sr));
      const rt = Math.exp(-1 / (Math.max(this.release._block(f0)[0], 1e-4) * sr));
      // Makeup: bring a full-scale signal back up part of the way (as browsers do).
      const makeup = Math.pow(10, (0.6 * -this._curve(0, T, K, R)) / 20);
      let env = this._env;
      for (let i = 0; i < BLOCK; i++) {
        const db = x[i] === 0 ? -1000 : 20 * Math.log10(Math.abs(x[i]));
        const want = Math.min(0, this._curve(db, T, K, R) - db);
        env = want < env ? at * env + (1 - at) * want : rt * env + (1 - rt) * want;
        out[i] = x[i] * Math.pow(10, env / 20) * makeup;
      }
      this._env = env;
      this._red = env;
      return out;
    }
  }

  // Radix-2 FFT magnitudes of a windowed frame.
  function fftMagnitudes(frame) {
    const n = frame.length, re = Float64Array.from(frame), im = new Float64Array(n);
    for (let i = 1, j = 0; i < n; i++) {
      let bit = n >> 1;
      for (; j & bit; bit >>= 1) j ^= bit;
      j ^= bit;
      if (i < j) { const t = re[i]; re[i] = re[j]; re[j] = t; }
    }
    for (let len = 2; len <= n; len <<= 1) {
      const ang = (-2 * Math.PI) / len, wr = Math.cos(ang), wi = Math.sin(ang);
      for (let i = 0; i < n; i += len) {
        let cr = 1, ci = 0;
        for (let k = 0; k < len / 2; k++) {
          const a = i + k, b = a + len / 2;
          const tr = re[b] * cr - im[b] * ci, ti = re[b] * ci + im[b] * cr;
          re[b] = re[a] - tr; im[b] = im[a] - ti;
          re[a] += tr; im[a] += ti;
          const ncr = cr * wr - ci * wi;
          ci = cr * wi + ci * wr;
          cr = ncr;
        }
      }
    }
    const mags = new Float64Array(n / 2);
    for (let k = 0; k < n / 2; k++) mags[k] = Math.hypot(re[k], im[k]) / n;
    return mags;
  }

  let analyserSalt = 0x51;
  class AnalyserNode extends AudioNode {
    constructor(ctx, o = {}) {
      super(ctx);
      this._fft = 2048;
      this.minDecibels = o.minDecibels ?? -100;
      this.maxDecibels = o.maxDecibels ?? -30;
      this.smoothingTimeConstant = o.smoothingTimeConstant ?? 0.8;
      this._ring = new Float32Array(32768);
      this._w = 0;
      this._smooth = new Float64Array(this._fft / 2);
      this._salt = analyserSalt++;
      if (o.fftSize) this.fftSize = o.fftSize;
      // An analyser runs whenever its context renders, even if nothing listens to its output.
      ctx._taps.add(this);
    }
    get fftSize() { return this._fft; }
    set fftSize(n) {
      n = Number(n);
      if (!(n >= 32 && n <= 32768 && (n & (n - 1)) === 0)) throw domErr("fftSize must be a power of two from 32 to 32768", "IndexSizeError");
      this._fft = n;
      this._smooth = new Float64Array(n / 2);
    }
    get frequencyBinCount() { return this._fft / 2; }
    _process(f0) {
      const x = this._in(f0), r = this._ring;
      for (let i = 0; i < BLOCK; i++) { r[this._w] = x[i]; this._w = (this._w + 1) % r.length; }
      return x;
    }
    _frame() {
      this._context._catchUp();
      const n = this._fft, r = this._ring, out = new Float32Array(n);
      for (let i = 0; i < n; i++) out[i] = r[(this._w - n + i + r.length) % r.length];
      return out;
    }
    _spectrum() {
      const fr = this._frame(), n = fr.length;
      // Blackman window.
      for (let i = 0; i < n; i++) {
        const a = (2 * Math.PI * i) / n;
        fr[i] *= 0.42 - 0.5 * Math.cos(a) + 0.08 * Math.cos(2 * a);
      }
      const m = fftMagnitudes(fr), s = this._smooth, k = Math.max(0, Math.min(1, this.smoothingTimeConstant));
      for (let i = 0; i < s.length; i++) s[i] = k * s[i] + (1 - k) * m[i];
      return s;
    }
    getFloatTimeDomainData(arr) {
      const fr = noiseFloats(this._frame(), this._salt);
      const n = Math.min(arr.length, fr.length);
      for (let i = 0; i < n; i++) arr[i] = fr[i];
    }
    getByteTimeDomainData(arr) {
      const fr = this._frame(), n = Math.min(arr.length, fr.length);
      const b = new Uint8Array(n);
      for (let i = 0; i < n; i++) b[i] = Math.max(0, Math.min(255, Math.floor(128 * (1 + fr[i]))));
      noiseBytes(b, this._salt);
      for (let i = 0; i < n; i++) arr[i] = b[i];
    }
    getFloatFrequencyData(arr) {
      const s = this._spectrum(), n = Math.min(arr.length, s.length);
      const db = new Float32Array(n);
      for (let i = 0; i < n; i++) db[i] = s[i] > 0 ? 20 * Math.log10(s[i]) : -Infinity;
      noiseFloats(db, this._salt ^ 0x777);
      for (let i = 0; i < n; i++) arr[i] = db[i];
    }
    getByteFrequencyData(arr) {
      const s = this._spectrum(), n = Math.min(arr.length, s.length);
      const lo = this.minDecibels, span = this.maxDecibels - lo || 1;
      const b = new Uint8Array(n);
      for (let i = 0; i < n; i++) {
        const db = s[i] > 0 ? 20 * Math.log10(s[i]) : -Infinity;
        b[i] = Math.max(0, Math.min(255, Math.floor((255 * (db - lo)) / span)));
      }
      noiseBytes(b, this._salt ^ 0x777);
      for (let i = 0; i < n; i++) arr[i] = b[i];
    }
  }

  // Nodes kept as pass-throughs (their sound arrives unchanged).
  class ChannelMergerNode extends AudioNode {}
  class ChannelSplitterNode extends AudioNode {}
  class ConvolverNode extends AudioNode {
    constructor(ctx, o = {}) { super(ctx); this.buffer = o.buffer ?? null; this.normalize = !(o.disableNormalization); }
  }
  class ScriptProcessorNode extends AudioNode {
    constructor(ctx, size = 4096, ins = 2, outs = 2) { super(ctx); this.bufferSize = size || 4096; this.onaudioprocess = null; }
  }

  // ---------------------------------------------------------------- buffers

  class AudioBuffer {
    constructor(o) {
      const ch = Number(o && o.numberOfChannels !== undefined ? o.numberOfChannels : 1);
      const len = Number(o && o.length), sr = Number(o && o.sampleRate);
      if (!(ch >= 1 && ch <= 32)) throw domErr("numberOfChannels must be 1–32", "NotSupportedError");
      if (!(len >= 1)) throw domErr("length must be at least 1", "NotSupportedError");
      if (!(sr >= 3000 && sr <= 768000)) throw domErr("sampleRate must be 3000–768000", "NotSupportedError");
      this._ch = Array.from({ length: ch }, () => new Float32Array(len));
      this.sampleRate = sr;
      this.length = len;
    }
    get numberOfChannels() { return this._ch.length; }
    get duration() { return this.length / this.sampleRate; }
    getChannelData(c) {
      if (!(c >= 0 && c < this._ch.length)) throw domErr("No such channel", "IndexSizeError");
      return this._ch[c];
    }
    copyFromChannel(dest, c, start = 0) {
      const src = this.getChannelData(c);
      for (let i = 0; i < dest.length && start + i < src.length; i++) dest[i] = src[start + i];
    }
    copyToChannel(src, c, start = 0) {
      const dst = this.getChannelData(c);
      for (let i = 0; i < src.length && start + i < dst.length; i++) dst[start + i] = src[i];
    }
  }

  // ---------------------------------------------------------------- contexts

  class BaseAudioContext extends g.EventTarget {
    constructor(sampleRate, channels) {
      super();
      this.sampleRate = sampleRate;
      this._frame = 0;
      this._taps = new Set();
      this._state = "running";
      this.destination = new AudioDestinationNode(this, channels);
      this.listener = { positionX: new AudioParam(this, 0), positionY: new AudioParam(this, 0), positionZ: new AudioParam(this, 0) };
      this.audioWorklet = { addModule: () => Promise.reject(domErr("AudioWorklet isn't supported", "NotSupportedError")) };
      this.onstatechange = null;
    }
    get state() { return this._state; }
    get currentTime() { return this._frame / this.sampleRate; }
    _later(fn) { setTimeout(fn, 0); }
    _catchUp() {}
    _setState(s) {
      if (this._state === s) return;
      this._state = s;
      this._later(() => this.dispatchEvent(new g.Event("statechange")));
    }
    // Render one block: the destination and any analysers off to the side.
    _renderBlock() {
      const f0 = this._frame;
      const out = this.destination._pull(f0);
      for (const t of this._taps) t._pull(f0);
      this._frame += BLOCK;
      return out;
    }
    createOscillator() { return new OscillatorNode(this); }
    createGain() { return new GainNode(this); }
    createBiquadFilter() { return new BiquadFilterNode(this); }
    createDynamicsCompressor() { return new DynamicsCompressorNode(this); }
    createAnalyser() { return new AnalyserNode(this); }
    createBufferSource() { return new AudioBufferSourceNode(this); }
    createConstantSource() { return new ConstantSourceNode(this); }
    createDelay(max = 1) { return new DelayNode(this, { maxDelayTime: max }); }
    createWaveShaper() { return new WaveShaperNode(this); }
    createStereoPanner() { return new StereoPannerNode(this); }
    createChannelMerger(n = 6) { const m = new ChannelMergerNode(this); m.numberOfInputs = n; return m; }
    createChannelSplitter(n = 6) { const s = new ChannelSplitterNode(this); s.numberOfOutputs = n; return s; }
    createConvolver() { return new ConvolverNode(this); }
    createScriptProcessor(size, ins, outs) { return new ScriptProcessorNode(this, size, ins, outs); }
    createPeriodicWave(real, imag, opts) { return new PeriodicWave(this, { real, imag, disableNormalization: !!(opts && opts.disableNormalization) }); }
    createBuffer(ch, len, sr) { return new AudioBuffer({ numberOfChannels: ch, length: len, sampleRate: sr }); }
    decodeAudioData(data, ok, fail) {
      const e = domErr("Decoding audio isn't supported yet", "EncodingError");
      if (typeof fail === "function") this._later(() => fail(e));
      return Promise.reject(e);
    }
  }

  class AudioContext extends BaseAudioContext {
    constructor(o = {}) {
      super(Number(o.sampleRate) || 48000, 2);
      this._t0 = g.performance ? g.performance.now() : Date.now();
      this._held = 0;
      this.baseLatency = 0.01;
      this.outputLatency = 0.02;
      this.latencyHint = o.latencyHint ?? "interactive";
    }
    _now() { return g.performance ? g.performance.now() : Date.now(); }
    // The clock runs while the context runs.
    get currentTime() {
      const ms = this._state === "running" ? this._now() - this._t0 : this._held;
      return Math.floor((ms / 1000) * this.sampleRate / BLOCK) * BLOCK / this.sampleRate;
    }
    // Render the graph up to the clock (at most the last two seconds of it).
    _catchUp() {
      const target = Math.round(this.currentTime * this.sampleRate);
      if (target - this._frame > this.sampleRate * 2) this._frame = Math.floor((target - this.sampleRate * 2) / BLOCK) * BLOCK;
      while (this._frame + BLOCK <= target) this._renderBlock();
    }
    getOutputTimestamp() { return { contextTime: this.currentTime, performanceTime: this._now() }; }
    resume() {
      if (this._state === "closed") return Promise.reject(domErr("The context is closed", "InvalidStateError"));
      if (this._state === "suspended") { this._t0 = this._now() - this._held; this._setState("running"); }
      return Promise.resolve();
    }
    suspend() {
      if (this._state === "running") { this._held = this._now() - this._t0; this._setState("suspended"); }
      return Promise.resolve();
    }
    close() {
      if (this._state === "running") this._held = this._now() - this._t0;
      this._setState("closed");
      return Promise.resolve();
    }
    createMediaElementSource() { return new AudioNode(this, 0, 1); }
    createMediaStreamSource() { return new AudioNode(this, 0, 1); }
    createMediaStreamDestination() { const n = new AudioNode(this, 1, 0); n.stream = null; return n; }
  }

  class OfflineAudioContext extends BaseAudioContext {
    constructor(a, b, c) {
      const o = typeof a === "object" && a !== null ? a : { numberOfChannels: a, length: b, sampleRate: c };
      const ch = o.numberOfChannels ?? 1;
      // Checks the arguments the way AudioBuffer does.
      const buf = new AudioBuffer({ numberOfChannels: ch, length: o.length, sampleRate: o.sampleRate });
      super(buf.sampleRate, ch);
      this._buf = buf;
      this.length = buf.length;
      this._state = "suspended";
      this._started = false;
      this.oncomplete = null;
    }
    startRendering() {
      if (this._started) return Promise.reject(domErr("startRendering() was already called", "InvalidStateError"));
      this._started = true;
      this._setState("running");
      return new Promise((resolve) => {
        this._later(() => {
          const buf = this._buf, len = buf.length;
          for (let f = 0; f < len; f += BLOCK) {
            const block = this._renderBlock();
            for (const c of buf._ch) for (let i = 0; i < BLOCK && f + i < len; i++) c[f + i] = block[i];
          }
          // What the page reads back carries the site's noise.
          buf._ch.forEach((c, i) => noiseFloats(c, 0xa0 + i));
          this._setState("closed");
          const ev = new g.Event("complete");
          ev.renderedBuffer = buf;
          this.dispatchEvent(ev);
          resolve(buf);
        });
      });
    }
    resume() { return Promise.reject(domErr("Can't resume an offline context that isn't suspended", "InvalidStateError")); }
    suspend() { return Promise.reject(domErr("Suspending offline rendering isn't supported", "NotSupportedError")); }
  }

  const exports = {
    AudioParam, AudioNode, AudioDestinationNode, AudioScheduledSourceNode, OscillatorNode, PeriodicWave,
    ConstantSourceNode, AudioBufferSourceNode, GainNode, StereoPannerNode, DelayNode, BiquadFilterNode,
    WaveShaperNode, DynamicsCompressorNode, AnalyserNode, ChannelMergerNode, ChannelSplitterNode,
    ConvolverNode, ScriptProcessorNode, AudioBuffer, BaseAudioContext, AudioContext, OfflineAudioContext,
  };
  for (const [k, v] of Object.entries(exports)) g[k] = v;
  g.webkitAudioContext = AudioContext;
  g.webkitOfflineAudioContext = OfflineAudioContext;
})(globalThis);
