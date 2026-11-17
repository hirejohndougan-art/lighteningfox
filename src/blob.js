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
