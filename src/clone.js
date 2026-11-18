// Structured clone: copies values the way postMessage does (Dates, RegExps, Maps, Sets,
// ArrayBuffers, typed arrays, Blobs, Errors, shared and circular references), and turns
// them into a string so they can cross to another thread (a Web Worker) and back.
// Functions, symbols and DOM nodes can't be copied: they throw a DataCloneError.
(function () {
  "use strict";
  const g = globalThis;

  function cloneError(what) {
    const msg = what + " could not be cloned.";
    return typeof g.DOMException === "function" ? new g.DOMException(msg, "DataCloneError") : new Error(msg);
  }

  function toB64(u8) {
    let s = "";
    for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
    return g.btoa(s);
  }
  function fromB64(b) {
    const s = g.atob(b);
    const u = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
    return u;
  }

  const TYPED = ["Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array",
    "Uint32Array", "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array", "DataView"];
  const ERRORS = ["Error", "EvalError", "RangeError", "ReferenceError", "SyntaxError", "TypeError", "URIError"];

  // value -> plain JSON-able tree. Objects get an index so repeats become references.
  function encode(root, ports) {
    const seen = new Map();
    function enc(v) {
      switch (typeof v) {
        case "boolean": return v;
        // Lone surrogates don't survive the trip through UTF-8: keep them as code units.
        case "string": return /[\ud800-\udfff]/.test(v) ? { $: "s16", v: Array.from({ length: v.length }, (_, k) => v.charCodeAt(k)) } : v;
        case "undefined": return { $: "u" };
        case "number":
          if (Number.isFinite(v) && !Object.is(v, -0)) return v;
          return { $: "n", v: Object.is(v, -0) ? "-0" : String(v) };
        case "bigint": return { $: "bi", v: v.toString() };
        case "symbol": throw cloneError("A symbol");
        case "function": throw cloneError("A function");
      }
      if (v === null) return null;
      if (ports && ports.has(v)) return { $: "port", p: ports.get(v) };
      if (seen.has(v)) return { $: "r", i: seen.get(v) };
      const i = seen.size;
      seen.set(v, i);
      if (v === g || (typeof v.nodeType === "number" && typeof v.nodeName === "string")) throw cloneError("A " + (v.nodeName || "window"));
      if (typeof g.MessagePort === "function" && v instanceof g.MessagePort) throw cloneError("A MessagePort that isn't in the transfer list");
      if (typeof Promise === "function" && v instanceof Promise) throw cloneError("A Promise");
      if (typeof WeakMap === "function" && (v instanceof WeakMap || v instanceof WeakSet)) throw cloneError("A WeakMap");
      if (Array.isArray(v)) {
        const out = { $: "a", i, l: v.length, v: {} };
        for (const k of Object.keys(v)) out.v[k] = enc(v[k]);
        return out;
      }
      if (v instanceof Date) return { $: "d", i, v: v.getTime() };
      if (v instanceof RegExp) return { $: "re", i, s: v.source, f: v.flags };
      if (v instanceof Map) { const out = { $: "m", i, v: [] }; for (const [a, b] of v) out.v.push([enc(a), enc(b)]); return out; }
      if (v instanceof Set) { const out = { $: "s", i, v: [] }; for (const a of v) out.v.push(enc(a)); return out; }
      if (v instanceof ArrayBuffer) return { $: "ab", i, v: toB64(new Uint8Array(v)) };
      if (ArrayBuffer.isView(v)) {
        const t = TYPED.find((n) => typeof g[n] === "function" && v instanceof g[n]) || "Uint8Array";
        const len = t === "DataView" ? v.byteLength : v.length;
        return { $: "ta", i, t, b: enc(v.buffer), o: v.byteOffset, l: len };
      }
      if (typeof g.Blob === "function" && v instanceof g.Blob) {
        const out = { $: "blob", i, v: toB64(v.__bytes()), t: v.type };
        if (typeof g.File === "function" && v instanceof g.File) { out.n = v.name; out.lm = v.lastModified; }
        return out;
      }
      if (v instanceof Error) {
        const n = ERRORS.includes(v.name) ? v.name : "Error";
        const out = { $: "e", i, n, m: String(v.message) };
        if (v.stack !== undefined) out.st = String(v.stack);
        return out;
      }
      if (v instanceof Boolean) return { $: "w", i, t: "b", v: v.valueOf() };
      if (v instanceof Number) return { $: "w", i, t: "n", v: enc(v.valueOf()) };
      if (v instanceof String) return { $: "w", i, t: "s", v: enc(v.valueOf()) };
      if (typeof BigInt === "function" && Object.prototype.toString.call(v) === "[object BigInt]") return { $: "w", i, t: "bi", v: v.valueOf().toString() };
      // A WebAssembly.Module goes as a token for its bytes (src/wasm/share.rs).
      if (typeof g.__wasmShare === "function" && g.WebAssembly && v instanceof g.WebAssembly.Module) return { $: "wasm", i, k: g.__wasmShare(v) };
      // Other WebAssembly objects can't be copied (a shared Memory could, but there are none).
      const wasmKind = g.WebAssembly && ["Instance", "Memory", "Table", "Global"].find((n) => typeof g.WebAssembly[n] === "function" && v instanceof g.WebAssembly[n]);
      if (wasmKind) throw cloneError("A WebAssembly." + wasmKind);
      if (typeof v.__clone === "function") return { $: "x", i, k: v.constructor.name, v: enc(v.__clone()) };
      const out = { $: "o", i, v: {} };
      for (const k of Object.keys(v)) out.v[k] = enc(v[k]);
      return out;
    }
    return enc(root);
  }

  function decode(root, ports) {
    const made = [];
    function dec(n) {
      if (n === null || typeof n !== "object") return n;
      switch (n.$) {
        case "u": return undefined;
        case "s16": return String.fromCharCode.apply(null, n.v);
        case "n": return n.v === "-0" ? -0 : Number(n.v);
        case "bi": return BigInt(n.v);
        case "r": return made[n.i];
        case "port": return ports ? ports[n.p] : undefined;
        case "a": { const a = new Array(n.l); made[n.i] = a; for (const k of Object.keys(n.v)) a[k] = dec(n.v[k]); return a; }
        case "d": return (made[n.i] = new Date(n.v));
        case "re": return (made[n.i] = new RegExp(n.s, n.f));
        case "m": { const m = new Map(); made[n.i] = m; for (const [a, b] of n.v) m.set(dec(a), dec(b)); return m; }
        case "s": { const s = new Set(); made[n.i] = s; for (const a of n.v) s.add(dec(a)); return s; }
        case "ab": return (made[n.i] = fromB64(n.v).buffer);
        case "ta": {
          const buf = dec(n.b);
          const C = g[n.t] || Uint8Array;
          return (made[n.i] = new C(buf, n.o, n.l));
        }
        case "blob": {
          const bytes = fromB64(n.v);
          return (made[n.i] = n.n !== undefined && typeof g.File === "function"
            ? new g.File([bytes], n.n, { type: n.t, lastModified: n.lm })
            : new g.Blob([bytes], { type: n.t }));
        }
        case "e": {
          const C = g[n.n] || Error;
          const e = new C(n.m);
          if (n.st !== undefined) { try { Object.defineProperty(e, "stack", { value: n.st, writable: true, configurable: true }); } catch (x) {} }
          return (made[n.i] = e);
        }
        case "w": return (made[n.i] = n.t === "b" ? Object(!!n.v) : n.t === "n" ? Object(dec(n.v)) : n.t === "bi" ? Object(BigInt(n.v)) : Object(String(dec(n.v))));
        case "x": {
          const C = g[n.k];
          const inner = dec(n.v);
          return (made[n.i] = C && typeof C.__unclone === "function" ? C.__unclone(inner) : inner);
        }
        case "wasm": return (made[n.i] = g.__wasmAdopt(n.k));
        case "o": { const o = {}; made[n.i] = o; for (const k of Object.keys(n.v)) o[k] = dec(n.v[k]); return o; }
      }
      return undefined;
    }
    return dec(root);
  }

  // Detach transferred ArrayBuffers so the sender can no longer use them, as browsers do.
  function detach(transfer) {
    if (!transfer) return;
    if (typeof transfer[Symbol.iterator] !== "function") throw new TypeError("transfer must be a sequence");
    for (const t of transfer) {
      if (t instanceof ArrayBuffer && typeof t.transfer === "function") { try { t.transfer(); } catch (e) {} }
    }
  }
  function transferList(opts) {
    if (opts == null) return undefined;
    if (Array.isArray(opts)) return opts;
    if (typeof opts === "object" && opts.transfer !== undefined) return opts.transfer;
    return undefined;
  }

  // `ports`: MessagePort -> id for ports being transferred to another thread.
  g.__cloneOut = (v, transfer, ports) => { const s = JSON.stringify(encode(v, ports)); detach(transfer); return s; };
  g.__cloneIn = (s) => decode(JSON.parse(s));
  g.__cloneTree = (tree, ports) => decode(tree, ports);
  g.__transferList = transferList;
  g.structuredClone = function structuredClone(v, opts) {
    if (arguments.length === 0) throw new TypeError("structuredClone needs a value");
    return decode(JSON.parse(g.__cloneOut(v, transferList(opts))));
  };
})();
