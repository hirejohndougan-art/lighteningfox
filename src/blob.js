// Blob and File: binary data held in memory (Uint8Array), with text(), arrayBuffer(),
// bytes(), slice() and stream-free reading through FileReader.
(function () {
  const g = globalThis;
  if (typeof g.Blob === "function") return;
  // UTF-8 without relying on TextEncoder/TextDecoder being set up yet.
  function utf8(str) {
    const out = [];
    for (let i = 0; i < str.length; i++) {
      let c = str.charCodeAt(i);
      if (c >= 0xd800 && c < 0xdc00 && i + 1 < str.length) {
        const d = str.charCodeAt(i + 1);
        if (d >= 0xdc00 && d < 0xe000) { c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00); i++; } else c = 0xfffd;
      } else if (c >= 0xd800 && c < 0xe000) c = 0xfffd;
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
      else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
      else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    }
    return new Uint8Array(out);
  }
  function fromUtf8(b) {
    let s = "";
    for (let i = 0; i < b.length; ) {
      const c = b[i++];
      let cp = 0xfffd;
      if (c < 0x80) cp = c;
      else if (c >= 0xc0 && c < 0xe0 && i < b.length) cp = ((c & 31) << 6) | (b[i++] & 63);
      else if (c >= 0xe0 && c < 0xf0 && i + 1 < b.length) { cp = ((c & 15) << 12) | ((b[i] & 63) << 6) | (b[i + 1] & 63); i += 2; }
      else if (c >= 0xf0 && i + 2 < b.length) { cp = ((c & 7) << 18) | ((b[i] & 63) << 12) | ((b[i + 1] & 63) << 6) | (b[i + 2] & 63); i += 3; }
      s += String.fromCodePoint(cp);
    }
    return s;
  }

  function bytesOf(part) {
    if (part instanceof Blob) return part.__bytes();
    if (part instanceof ArrayBuffer) return new Uint8Array(part.slice(0));
    if (ArrayBuffer.isView(part)) return new Uint8Array(part.buffer.slice(part.byteOffset, part.byteOffset + part.byteLength));
    return utf8(String(part));
  }

  class Blob {
    constructor(parts, opts) {
      if (parts !== undefined && (parts === null || typeof parts !== "object" || typeof parts[Symbol.iterator] !== "function")) {
        throw new TypeError("Blob parts must be a sequence.");
      }
      const chunks = Array.from(parts || [], bytesOf);
      const size = chunks.reduce((n, c) => n + c.length, 0);
      const data = new Uint8Array(size);
      let at = 0;
      for (const c of chunks) { data.set(c, at); at += c.length; }
      Object.defineProperty(this, "_data", { value: data });
      const type = opts && opts.type !== undefined ? String(opts.type) : "";
      Object.defineProperty(this, "_type", { value: /^[\x20-\x7e]*$/.test(type) ? type.toLowerCase() : "" });
    }
    get size() { return this._data.length; }
    get type() { return this._type; }
    __bytes() { return this._data; }
    slice(start, end, type) {
      const n = this.size;
      const rel = (v, d) => (v === undefined ? d : v < 0 ? Math.max(n + v, 0) : Math.min(v, n));
      const a = rel(start === undefined ? undefined : Math.trunc(start), 0);
      const b = rel(end === undefined ? undefined : Math.trunc(end), n);
      return new Blob([this._data.slice(a, Math.max(a, b))], { type: type === undefined ? "" : type });
    }
    text() { return Promise.resolve(fromUtf8(this._data)); }
    arrayBuffer() { return Promise.resolve(this._data.slice().buffer); }
    bytes() { return Promise.resolve(this._data.slice()); }
  }
  Object.defineProperty(Blob.prototype, Symbol.toStringTag, { value: "Blob" });

  class File extends Blob {
    constructor(parts, name, opts) {
      if (arguments.length < 2) throw new TypeError("File needs parts and a name.");
      super(parts, opts);
      Object.defineProperty(this, "_name", { value: String(name) });
      Object.defineProperty(this, "_mod", { value: opts && opts.lastModified !== undefined ? Number(opts.lastModified) : Date.now() });
    }
    get name() { return this._name; }
    get lastModified() { return this._mod; }
    get webkitRelativePath() { return ""; }
  }
  Object.defineProperty(File.prototype, Symbol.toStringTag, { value: "File" });

  class FileReader extends EventTarget {
    constructor() { super(); this.readyState = 0; this.result = null; this.error = null; }
    _read(blob, how) {
      if (!(blob instanceof Blob)) throw new TypeError("FileReader reads Blobs.");
      this.readyState = 1;
      setTimeout(() => {
        const d = blob.__bytes();
        if (how === "text") this.result = fromUtf8(d);
        else if (how === "buffer") this.result = d.slice().buffer;
        else if (how === "binary") this.result = Array.from(d, (b) => String.fromCharCode(b)).join("");
        else {
          let bin = "";
          for (const b of d) bin += String.fromCharCode(b);
          this.result = "data:" + (blob.type || "application/octet-stream") + ";base64," + btoa(bin);
        }
        this.readyState = 2;
        for (const t of ["load", "loadend"]) this.dispatchEvent(new Event(t));
      }, 0);
    }
    readAsText(b) { this._read(b, "text"); }
    readAsArrayBuffer(b) { this._read(b, "buffer"); }
    readAsBinaryString(b) { this._read(b, "binary"); }
    readAsDataURL(b) { this._read(b, "url"); }
    abort() {}
  }
  FileReader.EMPTY = 0; FileReader.LOADING = 1; FileReader.DONE = 2;

  // Base64 for binary strings (characters 0-255).
  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  if (typeof g.btoa !== "function") {
    g.btoa = (input) => {
      const s = String(input);
      if (/[^\x00-\xff]/.test(s)) throw new DOMException("The string has characters outside Latin-1.", "InvalidCharacterError");
      let out = "";
      for (let i = 0; i < s.length; i += 3) {
        const a = s.charCodeAt(i), b = s.charCodeAt(i + 1), c = s.charCodeAt(i + 2);
        const n = (a << 16) | ((b || 0) << 8) | (c || 0);
        out += B64[n >> 18] + B64[(n >> 12) & 63] + (i + 1 < s.length ? B64[(n >> 6) & 63] : "=") + (i + 2 < s.length ? B64[n & 63] : "=");
      }
      return out;
    };
  }
  if (typeof g.atob !== "function") {
    g.atob = (input) => {
      let s = String(input).replace(/[\t\n\f\r ]/g, "");
      if (s.length % 4 === 0) s = s.replace(/==?$/, "");
      if (s.length % 4 === 1 || /[^A-Za-z0-9+/]/.test(s)) throw new DOMException("The string is not valid base64.", "InvalidCharacterError");
      let out = "", bits = 0, acc = 0;
      for (const ch of s) {
        acc = (acc << 6) | B64.indexOf(ch);
        bits += 6;
        if (bits >= 8) { bits -= 8; out += String.fromCharCode((acc >> bits) & 255); }
      }
      return out;
    };
  }

  if (typeof g.TextEncoder !== "function") {
    g.TextEncoder = class TextEncoder {
      get encoding() { return "utf-8"; }
      encode(s) { return utf8(s === undefined ? "" : String(s)); }
      encodeInto(s, dest) {
        const b = utf8(String(s));
        let n = Math.min(b.length, dest.length);
        // Don't split a character.
        while (n > 0 && n < b.length && (b[n] & 0xc0) === 0x80) n--;
        dest.set(b.subarray(0, n));
        return { read: fromUtf8(b.subarray(0, n)).length, written: n };
      }
    };
  }
  if (typeof g.TextDecoder !== "function") {
    g.TextDecoder = class TextDecoder {
      constructor(label, opts) {
        const l = label === undefined ? "utf-8" : String(label).trim().toLowerCase();
        const names = { "utf-8": "utf-8", utf8: "utf-8", "unicode-1-1-utf-8": "utf-8", "iso-8859-1": "windows-1252", latin1: "windows-1252", "windows-1252": "windows-1252", ascii: "windows-1252", "us-ascii": "windows-1252" };
        if (!names[l]) throw new RangeError("Unsupported encoding: " + label);
        this._enc = names[l];
        this.fatal = !!(opts && opts.fatal);
        this.ignoreBOM = !!(opts && opts.ignoreBOM);
      }
      get encoding() { return this._enc; }
      decode(input) {
        if (input === undefined) return "";
        let b = input instanceof ArrayBuffer ? new Uint8Array(input) : ArrayBuffer.isView(input) ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength) : null;
        if (!b) throw new TypeError("decode() needs an ArrayBuffer or a typed array.");
        if (this._enc !== "utf-8") return Array.from(b, (c) => String.fromCharCode(c)).join("");
        if (!this.ignoreBOM && b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf) b = b.subarray(3);
        const s = fromUtf8(b);
        if (this.fatal && s.includes("\ufffd") && !utf8(s).every((x, i) => x === b[i])) throw new TypeError("The data is not valid UTF-8.");
        return s;
      }
    };
  }

  Object.assign(g, { Blob, File, FileReader });
})();

// Web Crypto: crypto.getRandomValues, crypto.randomUUID and crypto.subtle.digest (SHA-1/256/
// 384/512). Other subtle operations (keys, signing, encryption) reject with NotSupportedError.
(function () {
  "use strict";
  const g = globalThis;
  if (g.crypto && g.crypto.getRandomValues) return;
  const illegal = (name) => { const C = { [name]: function () { throw new TypeError("Illegal constructor"); } }[name]; return C; };
  const Crypto = illegal("Crypto"), SubtleCrypto = illegal("SubtleCrypto"), CryptoKey = illegal("CryptoKey");
  const bytesOf = (src) => {
    if (src instanceof ArrayBuffer) return new Uint8Array(src);
    if (ArrayBuffer.isView(src)) return new Uint8Array(src.buffer, src.byteOffset, src.byteLength);
    throw new TypeError("Expected an ArrayBuffer or a typed array");
  };
  const toStr = (u8) => { let s = ""; for (let i = 0; i < u8.length; i += 8192) s += String.fromCharCode.apply(null, u8.subarray(i, i + 8192)); return s; };
  const intArrays = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array", "BigInt64Array", "BigUint64Array"];

  const subtle = Object.create(SubtleCrypto.prototype);
  subtle.digest = function digest(algorithm, data) {
    try {
      const name = typeof algorithm === "string" ? algorithm : algorithm && algorithm.name;
      const out = __digest(String(name), toStr(bytesOf(data)));
      if (out === undefined) return Promise.reject(new DOMException("Unrecognized algorithm name", "NotSupportedError"));
      const u8 = new Uint8Array(out.length);
      for (let i = 0; i < out.length; i++) u8[i] = out.charCodeAt(i);
      return Promise.resolve(u8.buffer);
    } catch (e) { return Promise.reject(e); }
  };
  for (const m of ["encrypt", "decrypt", "sign", "verify", "generateKey", "deriveKey", "deriveBits", "importKey", "exportKey", "wrapKey", "unwrapKey"]) {
    subtle[m] = function () { return Promise.reject(new DOMException("crypto.subtle." + m + " isn't supported yet", "NotSupportedError")); };
  }

  const crypto = Object.create(Crypto.prototype);
  crypto.getRandomValues = function getRandomValues(a) {
    if (!ArrayBuffer.isView(a) || !intArrays.includes(Object.prototype.toString.call(a).slice(8, -1))) {
      throw new DOMException("getRandomValues needs an integer typed array", "TypeMismatchError");
    }
    if (a.byteLength > 65536) throw new DOMException("getRandomValues can fill at most 65536 bytes", "QuotaExceededError");
    const r = __randomBytes(a.byteLength), u8 = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
    for (let i = 0; i < u8.length; i++) u8[i] = r.charCodeAt(i);
    return a;
  };
  crypto.randomUUID = function randomUUID() {
    const b = crypto.getRandomValues(new Uint8Array(16));
    b[6] = (b[6] & 0x0f) | 0x40; b[8] = (b[8] & 0x3f) | 0x80;
    const h = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
    return h.slice(0, 8) + "-" + h.slice(8, 12) + "-" + h.slice(12, 16) + "-" + h.slice(16, 20) + "-" + h.slice(20);
  };
  Object.defineProperty(crypto, "subtle", { get: () => subtle, enumerable: true, configurable: true });
  Object.defineProperty(g, "crypto", { get: () => crypto, enumerable: true, configurable: true });
  Object.assign(g, { Crypto, SubtleCrypto, CryptoKey });
})();

// User Timing (performance.mark/measure and their entries) and PerformanceObserver. Only
// "mark" and "measure" entries are recorded; observers of other types never get entries.
(function () {
  "use strict";
  const g = globalThis, perf = g.performance;
  if (!perf || g.PerformanceObserver) return;
  const entries = [], observers = new Set();
  class PerformanceEntry {
    constructor(name, entryType, startTime, duration, detail) {
      Object.assign(this, { name, entryType, startTime, duration });
      if (detail !== undefined) this.detail = detail;
    }
    toJSON() { return { name: this.name, entryType: this.entryType, startTime: this.startTime, duration: this.duration }; }
  }
  class PerformanceMark extends PerformanceEntry {}
  class PerformanceMeasure extends PerformanceEntry {}
  const record = (e) => {
    entries.push(e);
    if (entries.length > 10000) entries.shift();
    for (const o of observers) if (o._types.has(e.entryType)) { o._queue.push(e); o._schedule(); }
    return e;
  };
  const timeOf = (m) => {
    if (m === undefined) return perf.now();
    if (typeof m === "number") return m;
    for (let i = entries.length - 1; i >= 0; i--) if (entries[i].entryType === "mark" && entries[i].name === m) return entries[i].startTime;
    throw new DOMException(`The mark '${m}' does not exist.`, "SyntaxError");
  };
  perf.timeOrigin = perf.timeOrigin || Date.now() - perf.now();
  perf.mark = function mark(name, opts) {
    return record(new PerformanceMark(String(name), "mark", opts && opts.startTime !== undefined ? +opts.startTime : perf.now(), 0, opts ? opts.detail : undefined));
  };
  perf.measure = function measure(name, start, end) {
    let s, e, detail;
    if (start && typeof start === "object") {
      detail = start.detail;
      s = timeOf(start.start); e = start.end !== undefined ? timeOf(start.end) : start.duration !== undefined ? s + start.duration : perf.now();
    } else { s = start === undefined ? 0 : timeOf(start); e = timeOf(end); }
    return record(new PerformanceMeasure(String(name), "measure", s, e - s, detail));
  };
  const pick = (type, name) => entries.filter((x) => (type === undefined || x.entryType === type) && (name === undefined || x.name === name));
  perf.getEntries = () => pick();
  perf.getEntriesByType = (t) => pick(String(t));
  perf.getEntriesByName = (n, t) => pick(t === undefined ? undefined : String(t), String(n));
  const clear = (type) => function (name) {
    for (let i = entries.length - 1; i >= 0; i--) if (entries[i].entryType === type && (name === undefined || entries[i].name === name)) entries.splice(i, 1);
  };
  perf.clearMarks = clear("mark");
  perf.clearMeasures = clear("measure");
  perf.clearResourceTimings = function () {};
  perf.setResourceTimingBufferSize = function () {};
  perf.toJSON = () => ({ timeOrigin: perf.timeOrigin });

  class PerformanceObserverEntryList {
    constructor(list) { this._l = list; }
    getEntries() { return this._l.slice(); }
    getEntriesByType(t) { return this._l.filter((x) => x.entryType === t); }
    getEntriesByName(n, t) { return this._l.filter((x) => x.name === n && (t === undefined || x.entryType === t)); }
  }
  class PerformanceObserver {
    constructor(callback) {
      if (typeof callback !== "function") throw new TypeError("PerformanceObserver needs a callback");
      this._cb = callback; this._types = new Set(); this._queue = []; this._pending = false;
    }
    static get supportedEntryTypes() { return ["mark", "measure"]; }
    observe(opts) {
      opts = opts || {};
      const types = opts.type !== undefined ? [String(opts.type)] : Array.from(opts.entryTypes || []).map(String);
      if (!types.length) throw new TypeError("observe() needs type or entryTypes");
      types.forEach((t) => this._types.add(t));
      observers.add(this);
      if (opts.buffered) { for (const e of entries) if (types.includes(e.entryType)) this._queue.push(e); if (this._queue.length) this._schedule(); }
    }
    disconnect() { observers.delete(this); this._queue = []; this._types.clear(); }
    takeRecords() { const q = this._queue; this._queue = []; return q; }
    _schedule() {
      if (this._pending) return;
      this._pending = true;
      setTimeout(() => {
        this._pending = false;
        if (!this._queue.length) return;
        const list = new PerformanceObserverEntryList(this.takeRecords());
        try { this._cb.call(this, list, this); } catch (e) { console.error(String(e)); if (g.__onListenerError) g.__onListenerError(e); }
      }, 0);
    }
  }
  Object.assign(g, { PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList });
})();
