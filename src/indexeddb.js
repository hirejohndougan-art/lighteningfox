// IndexedDB for Ironswift: databases, object stores, indexes, key ranges, cursors and
// transactions, kept per site (origin) in <profile>/indexeddb/<origin>.json.
//
// How it works:
// - Everything for a site is held in memory once loaded; a read-write transaction saves the
//   whole site file when it commits (an aborted one rolls back to a snapshot).
// - Values are stored "encoded": a JSON-safe form that keeps Dates, Maps, Sets, binary data,
//   undefined, NaN and so on (this is also the structured clone). Keys are encoded as
//   [type, value] pairs that sort in IndexedDB key order.
// - Transactions run one at a time in the order they were created. Requests run one per
//   step, with a few microtask turns in between, so promise-based wrappers (like the `idb`
//   library) can queue the next request before the transaction commits. A transaction
//   commits once a full task passes with nothing queued.
(function () {
  const g = globalThis;
  const ORIGIN = location.origin === "null" || !location.origin ? "file" : location.origin;

  const err = (name, msg) => new DOMException(msg || name, name);

  // ------------------------------------------------------------ values (structured clone)

  function encode(v, seen) {
    if (v === null || typeof v === "boolean" || typeof v === "string") return v;
    if (typeof v === "number") return Number.isFinite(v) && !Object.is(v, -0) ? v : { $: "n", v: Object.is(v, -0) ? "-0" : String(v) };
    if (v === undefined) return { $: "u" };
    if (typeof v === "bigint") return { $: "bi", v: String(v) };
    if (typeof v === "function" || typeof v === "symbol") throw err("DataCloneError", "The value can't be stored (it contains a " + typeof v + ").");
    seen = seen || new Set();
    if (seen.has(v)) throw err("DataCloneError", "The value can't be stored (it refers to itself).");
    seen.add(v);
    try {
      if (v instanceof Date) return { $: "d", v: v.getTime() };
      if (v instanceof RegExp) return { $: "r", s: v.source, f: v.flags };
      if (v instanceof ArrayBuffer) return { $: "ab", v: Array.from(new Uint8Array(v)) };
      if (ArrayBuffer.isView(v)) {
        const bytes = new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
        return { $: "ta", t: v.constructor.name, v: Array.from(bytes) };
      }
      if (typeof Blob === "function" && v instanceof Blob) {
        const o = { $: "blob", t: v.type, v: Array.from(v.__bytes()) };
        if (typeof File === "function" && v instanceof File) { o.$ = "file"; o.n = v.name; o.m = v.lastModified; }
        return o;
      }
      if (v instanceof Map) return { $: "m", v: Array.from(v, ([k, x]) => [encode(k, seen), encode(x, seen)]) };
      if (v instanceof Set) return { $: "s", v: Array.from(v, (x) => encode(x, seen)) };
      if (Array.isArray(v)) return { $: "a", v: v.map((x) => encode(x, seen)) };
      if (v instanceof Error) return { $: "e", n: v.name, m: v.message };
      if (v instanceof Boolean || v instanceof Number || v instanceof String) return { $: "w", v: encode(v.valueOf(), seen) };
      if (typeof Node === "function" && v instanceof Node) throw err("DataCloneError", "A page element can't be stored.");
      // WebAssembly objects can't be stored (a Module only goes to workers, with postMessage).
      if (g.WebAssembly && ["Module", "Instance", "Memory", "Table", "Global"].some((n) => typeof g.WebAssembly[n] === "function" && v instanceof g.WebAssembly[n]))
        throw err("DataCloneError", "A WebAssembly object can't be stored.");
      const o = {};
      for (const k of Object.keys(v)) o[k] = encode(v[k], seen);
      return { $: "o", v: o };
    } finally {
      seen.delete(v);
    }
  }

  function decode(e) {
    if (e === null || typeof e !== "object") return e;
    switch (e.$) {
      case "o": { const o = {}; for (const k of Object.keys(e.v)) o[k] = decode(e.v[k]); return o; }
      case "a": return e.v.map(decode);
      case "n": return e.v === "-0" ? -0 : Number(e.v);
      case "u": return undefined;
      case "bi": return BigInt(e.v);
      case "d": return new Date(e.v);
      case "r": return new RegExp(e.s, e.f);
      case "ab": return new Uint8Array(e.v).buffer;
      case "ta": {
        const C = g[e.t] || Uint8Array;
        const buf = new Uint8Array(e.v).buffer;
        return C === DataView ? new DataView(buf) : new C(buf);
      }
      case "blob": return new Blob([new Uint8Array(e.v)], { type: e.t });
      case "file": return new File([new Uint8Array(e.v)], e.n, { type: e.t, lastModified: e.m });
      case "m": return new Map(e.v.map(([k, x]) => [decode(k), decode(x)]));
      case "s": return new Set(e.v.map(decode));
      case "e": { const x = new (g[e.n] && g[e.n].prototype instanceof Error ? g[e.n] : Error)(e.m); x.name = e.n; return x; }
      case "w": return Object(decode(e.v));
      default: return e;
    }
  }

  // ------------------------------------------------------------ keys

  const RANK = { n: 0, d: 1, s: 2, b: 3, a: 4 };

  /** The encoded key, or undefined when `v` isn't a valid key. */
  function keyOf(v, seen) {
    if (typeof v === "number") return Number.isNaN(v) ? undefined : ["n", v];
    if (typeof v === "string") return ["s", v];
    if (v instanceof Date) { const t = v.getTime(); return Number.isNaN(t) ? undefined : ["d", t]; }
    if (v instanceof ArrayBuffer) return v.detached ? undefined : ["b", Array.from(new Uint8Array(v))];
    if (ArrayBuffer.isView(v) && v.buffer.detached) return undefined;
    if (ArrayBuffer.isView(v)) return ["b", Array.from(new Uint8Array(v.buffer, v.byteOffset, v.byteLength))];
    if (Array.isArray(v)) {
      seen = seen || new Set();
      if (seen.has(v)) return undefined;
      seen.add(v);
      const out = [];
      for (let i = 0; i < v.length; i++) {
        if (!(i in v)) return undefined;
        const k = keyOf(v[i], seen);
        if (k === undefined) return undefined;
        out.push(k);
      }
      seen.delete(v);
      return ["a", out];
    }
    return undefined;
  }

  function mustKey(v) {
    const k = keyOf(v);
    if (k === undefined) throw err("DataError", "The value is not a valid key.");
    return k;
  }

  function keyValue(k) {
    switch (k[0]) {
      case "n": return k[1];
      case "s": return k[1];
      case "d": return new Date(k[1]);
      case "b": return new Uint8Array(k[1]).buffer;
      case "a": return k[1].map(keyValue);
    }
  }

  function cmp(a, b) {
    if (a[0] !== b[0]) return RANK[a[0]] < RANK[b[0]] ? -1 : 1;
    switch (a[0]) {
      case "n": case "d": return a[1] === b[1] ? 0 : a[1] < b[1] ? -1 : 1;
      case "s": return a[1] === b[1] ? 0 : a[1] < b[1] ? -1 : 1;
      case "b": case "a": {
        const x = a[1], y = b[1];
        for (let i = 0; i < Math.min(x.length, y.length); i++) {
          const c = a[0] === "b" ? x[i] - y[i] : cmp(x[i], y[i]);
          if (c) return c < 0 ? -1 : 1;
        }
        return x.length === y.length ? 0 : x.length < y.length ? -1 : 1;
      }
    }
    return 0;
  }

  // ------------------------------------------------------------ key paths

  function validPath(p) {
    if (p === "") return true;
    return typeof p === "string" && p.split(".").every((s) => /^[A-Za-z_$][\w$]*$/.test(s));
  }

  function checkKeyPath(kp) {
    if (kp === null || kp === undefined) return null;
    if (Array.isArray(kp)) {
      if (!kp.length || !kp.every((p) => validPath(String(p)))) throw err("SyntaxError", "Invalid key path.");
      return kp.map(String);
    }
    kp = String(kp);
    if (!validPath(kp)) throw err("SyntaxError", "Invalid key path.");
    return kp;
  }

  /** The value at a key path (an array of them for an array path), or undefined. */
  function evalPath(value, kp) {
    if (Array.isArray(kp)) {
      const out = [];
      for (const p of kp) {
        const v = evalPath(value, p);
        if (v === undefined) return undefined;
        out.push(v);
      }
      return out;
    }
    if (kp === "") return value;
    let v = value;
    for (const part of kp.split(".")) {
      if (v === null || v === undefined) return undefined;
      if (typeof v === "string" && part === "length") v = v.length;
      else if (typeof v !== "object") return undefined;
      else if (!Object.prototype.hasOwnProperty.call(v, part) && !(Array.isArray(v) && part === "length")) return undefined;
      else v = v[part];
    }
    return v;
  }

  function injectKey(value, kp, key) {
    const parts = kp.split(".");
    let o = value;
    for (let i = 0; i < parts.length - 1; i++) {
      if (o[parts[i]] === undefined) o[parts[i]] = {};
      o = o[parts[i]];
      if (typeof o !== "object" || o === null) throw err("DataError", "The key can't be put into the value.");
    }
    o[parts[parts.length - 1]] = key;
  }

  /** Can the generated key be written into `value` at `kp`? */
  function canInject(value, kp) {
    let o = value;
    if (typeof o !== "object" || o === null) return false;
    const parts = kp.split(".");
    for (let i = 0; i < parts.length - 1; i++) {
      const n = o[parts[i]];
      if (n === undefined) return true;
      if (typeof n !== "object" || n === null) return false;
      o = n;
    }
    return true;
  }

  // ------------------------------------------------------------ sorted arrays

  /** First index whose item is >= key (by `get` and `cmp`). */
  function lower(arr, key, get) {
    let lo = 0, hi = arr.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (cmp(get(arr[mid]), key) < 0) lo = mid + 1; else hi = mid;
    }
    return lo;
  }
  /** First index whose item is > key. */
  function upper(arr, key, get) {
    let lo = 0, hi = arr.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (cmp(get(arr[mid]), key) <= 0) lo = mid + 1; else hi = mid;
    }
    return lo;
  }
  // Index entries sort by (index key, primary key).
  const entCmp = (a, b) => cmp(a.k, b.k) || cmp(a.p, b.p);
  function entLower(arr, e) {
    let lo = 0, hi = arr.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (entCmp(arr[mid], e) < 0) lo = mid + 1; else hi = mid;
    }
    return lo;
  }

  // ------------------------------------------------------------ the site's data

  // dbs: name -> { name, version, stores: Map(name -> store) }
  // store: { name, keyPath, autoIncrement, current, recs: [{k, v}], idx: Map(name -> index) }
  // index: { name, keyPath, unique, multiEntry, ents: [{k, p}] }
  let dbs = null;

  function load() {
    if (dbs) return dbs;
    dbs = new Map();
    let data = {};
    try { data = JSON.parse(__idbLoad(ORIGIN)) || {}; } catch (e) { data = {}; }
    for (const [name, d] of Object.entries(data)) {
      const db = { name, version: d.version, stores: new Map() };
      for (const [sn, s] of Object.entries(d.stores || {})) {
        const st = { name: sn, keyPath: s.keyPath, autoIncrement: !!s.autoIncrement, current: s.current || 1, recs: s.recs || [], idx: new Map() };
        for (const [iname, ix] of Object.entries(s.idx || {})) {
          const index = { name: iname, keyPath: ix.keyPath, unique: !!ix.unique, multiEntry: !!ix.multiEntry, ents: [] };
          for (const r of st.recs) addEntries(index, r.k, decode(r.v));
          st.idx.set(iname, index);
        }
        db.stores.set(sn, st);
      }
      dbs.set(name, db);
    }
    return dbs;
  }

  function save() {
    const out = {};
    for (const [name, db] of load()) {
      const stores = {};
      for (const [sn, s] of db.stores) {
        const idx = {};
        for (const [iname, ix] of s.idx) idx[iname] = { keyPath: ix.keyPath, unique: ix.unique, multiEntry: ix.multiEntry };
        stores[sn] = { keyPath: s.keyPath, autoIncrement: s.autoIncrement, current: s.current, recs: s.recs, idx };
      }
      out[name] = { version: db.version, stores };
    }
    return __idbSave(ORIGIN, JSON.stringify(out));
  }

  function indexKeys(index, value) {
    const v = evalPath(value, index.keyPath);
    if (v === undefined) return [];
    if (index.multiEntry && Array.isArray(v)) {
      const out = [];
      for (const x of v) {
        const k = keyOf(x);
        if (k !== undefined && !out.some((o) => cmp(o, k) === 0)) out.push(k);
      }
      return out;
    }
    const k = keyOf(v);
    return k === undefined ? [] : [k];
  }

  function addEntries(index, p, value) {
    for (const k of indexKeys(index, value)) {
      const e = { k, p };
      index.ents.splice(entLower(index.ents, e), 0, e);
    }
  }

  function removeEntries(index, p) {
    index.ents = index.ents.filter((e) => cmp(e.p, p) !== 0);
  }

  // Deep enough copy for rolling back: arrays are copied, records and entries (never
  // changed in place) are shared.
  function snapshotStore(s) {
    const idx = new Map();
    for (const [n, ix] of s.idx) idx.set(n, { ...ix, ents: ix.ents.slice() });
    return { ...s, recs: s.recs.slice(), idx };
  }

  // ------------------------------------------------------------ key ranges

  class IDBKeyRange {
    constructor(lowerK, upperK, lowerOpen, upperOpen) {
      this._l = lowerK; this._u = upperK;
      this.lowerOpen = !!lowerOpen; this.upperOpen = !!upperOpen;
    }
    get lower() { return this._l === null ? undefined : keyValue(this._l); }
    get upper() { return this._u === null ? undefined : keyValue(this._u); }
    static only(v) { const k = mustKey(v); return new IDBKeyRange(k, k, false, false); }
    static lowerBound(v, open) { return new IDBKeyRange(mustKey(v), null, open, true); }
    static upperBound(v, open) { return new IDBKeyRange(null, mustKey(v), true, open); }
    static bound(a, b, lo, uo) {
      const l = mustKey(a), u = mustKey(b);
      const c = cmp(l, u);
      if (c > 0 || (c === 0 && (lo || uo))) throw err("DataError", "The lower bound is above the upper bound.");
      return new IDBKeyRange(l, u, lo, uo);
    }
    _has(k) {
      if (this._l !== null) { const c = cmp(k, this._l); if (c < 0 || (c === 0 && this.lowerOpen)) return false; }
      if (this._u !== null) { const c = cmp(k, this._u); if (c > 0 || (c === 0 && this.upperOpen)) return false; }
      return true;
    }
    includes(v) { return this._has(mustKey(v)); }
  }

  /** A query argument (a key or a range) as a range; null = everything. */
  function toRange(q, required) {
    if (q === undefined || q === null) {
      if (required) throw err("DataError", "A key or key range is needed.");
      return null;
    }
    if (q instanceof IDBKeyRange) return q;
    return IDBKeyRange.only(q);
  }

  /** Indices [from, to) of the items of a sorted array inside a range. */
  function span(arr, range, get) {
    if (!range) return [0, arr.length];
    const from = range._l === null ? 0 : range.lowerOpen ? upper(arr, range._l, get) : lower(arr, range._l, get);
    const to = range._u === null ? arr.length : range.upperOpen ? lower(arr, range._u, get) : upper(arr, range._u, get);
    return [from, Math.max(from, to)];
  }

  // ------------------------------------------------------------ string lists

  function stringList(names) {
    const a = Array.from(names).sort();
    a.contains = (s) => a.includes(String(s));
    a.item = (i) => (i >= 0 && i < a.length ? a[i] : null);
    return Object.freeze(a);
  }

  // ------------------------------------------------------------ events

  class IDBVersionChangeEvent extends Event {
    constructor(type, init) {
      super(type, init);
      init = init || {};
      this.oldVersion = init.oldVersion || 0;
      this.newVersion = init.newVersion === undefined ? null : init.newVersion;
    }
  }

  // Fire an event; ev.threw tells whether a listener threw (that aborts a transaction).
  function fire(target, type, init, parents) {
    const ev = new (init && "oldVersion" in init ? IDBVersionChangeEvent : Event)(type, init);
    const before = g.__onListenerError;
    let threw = false;
    g.__onListenerError = () => { threw = true; };
    try {
      target.dispatchEvent(ev);
      // Bubble from a request to its transaction and database.
      if (ev.bubbles && parents) {
        for (const p of parents) {
          if (ev._stop || !p) break;
          ev.eventPhase = 3;
          EventTarget.prototype.__fire.call(p, ev);
        }
      }
    } finally {
      g.__onListenerError = before;
    }
    ev.threw = threw;
    return ev;
  }

  // ------------------------------------------------------------ requests

  class IDBRequest extends EventTarget {
    constructor(source, tx) {
      super();
      this.source = source || null;
      this.transaction = tx || null;
      this.readyState = "pending";
      this._result = undefined;
      this._error = null;
      this.onsuccess = null;
      this.onerror = null;
    }
    get result() {
      if (this.readyState !== "done") throw err("InvalidStateError", "The request isn't finished.");
      return this._result;
    }
    get error() {
      if (this.readyState !== "done") throw err("InvalidStateError", "The request isn't finished.");
      return this._error;
    }
  }

  class IDBOpenDBRequest extends IDBRequest {
    constructor() { super(null, null); this.onupgradeneeded = null; this.onblocked = null; }
  }

  // ------------------------------------------------------------ the transaction queue

  const queue = []; // transactions (and open/delete jobs) waiting to run, in order
  let running = null;
  let pumping = false;

  function pump() {
    if (pumping) return;
    pumping = true;
    setTimeout(() => {
      pumping = false;
      if (running || !queue.length) return;
      running = queue.shift();
      running._start();
    }, 0);
  }

  function finished(job) {
    if (running === job) running = null;
    pump();
  }

  // A few microtask turns: lets promise continuations queue their next request.
  function afterMicrotasks(fn) {
    let p = Promise.resolve();
    for (let i = 0; i < 6; i++) p = p.then(() => {});
    p.then(fn);
  }

  class IDBTransaction extends EventTarget {
    constructor(db, names, mode, durability) {
      super();
      this.db = db;
      this.mode = mode;
      this.durability = durability || "default";
      this._names = names;
      this.error = null;
      this.oncomplete = null;
      this.onerror = null;
      this.onabort = null;
      this._reqs = [];
      this._state = "active"; // active -> committing/finished/aborted
      this._snapshot = null;
      this._stores = new Map();
      this._dirty = false;
      this._commitAsked = false;
    }
    get objectStoreNames() { return stringList(this.mode === "versionchange" ? this.db._data.stores.keys() : this._names); }
    objectStore(name) {
      if (this._state === "finished" || this._state === "aborted") throw err("InvalidStateError", "The transaction has finished.");
      if (!this._names.includes(name) && !(this.mode === "versionchange" && this.db._data.stores.has(name))) throw err("NotFoundError", "No object store named " + name + " in this transaction.");
      if (!this.db._data.stores.has(name)) throw err("NotFoundError", "No object store named " + name + ".");
      let s = this._stores.get(name);
      if (!s) { s = new IDBObjectStore(this, name); this._stores.set(name, s); }
      return s;
    }
    _active() {
      if (this._state !== "active") throw err("TransactionInactiveError", "The transaction is not active.");
    }
    _write() {
      if (this.mode === "readonly") throw err("ReadOnlyError", "The transaction is read-only.");
    }
    // Before the first change, keep a copy of what this transaction may change.
    _touch() {
      if (this._snapshot) return;
      const data = this.db._data;
      if (this.mode === "versionchange") {
        const stores = new Map();
        for (const [n, s] of data.stores) stores.set(n, snapshotStore(s));
        this._snapshot = { version: this._oldVersion, stores };
      } else {
        const stores = new Map();
        for (const n of this._names) if (data.stores.has(n)) stores.set(n, snapshotStore(data.stores.get(n)));
        this._snapshot = { stores };
      }
      this._dirty = true;
    }
    _add(req, op) {
      this._active();
      this._reqs.push({ req, op });
      return req;
    }
    _start() {
      this._step();
    }
    _step() {
      if (this._state === "aborted" || this._state === "finished") return;
      if (this._reqs.length) {
        const { req, op } = this._reqs.shift();
        let result, error = null;
        try { result = op(); } catch (e) { error = e instanceof DOMException ? e : err("UnknownError", String(e)); }
        req.readyState = "done";
        if (error) {
          req._error = error;
          req._result = undefined;
          const ev = fire(req, "error", { bubbles: true, cancelable: true }, [this, this.db]);
          if (this._state === "active") {
            if (ev.threw) this._abort(err("AbortError", "An error event handler threw."));
            else if (!ev.defaultPrevented) this._abort(error);
          }
        } else {
          req._result = result;
          const ev = fire(req, "success");
          if (ev.threw && this._state === "active") this._abort(err("AbortError", "A success event handler threw."));
        }
        afterMicrotasks(() => this._step());
      } else {
        // Nothing queued: commit once a whole task passes with nothing new.
        setTimeout(() => {
          if (this._state !== "active") return;
          if (this._reqs.length) this._step(); else this._commit();
        }, 0);
      }
    }
    _commit() {
      this._state = "finished";
      if (this._dirty && !save()) {
        this._rollback();
        this.error = err("QuotaExceededError", "The site's storage quota has been exceeded.");
        fire(this, "abort", { bubbles: true }, [this.db]);
        this._done();
        return;
      }
      fire(this, "complete");
      this._done();
    }
    _rollback() {
      if (!this._snapshot) return;
      const data = this.db._data;
      if (this.mode === "versionchange") {
        data.stores = this._snapshot.stores;
        data.version = this._snapshot.version;
        if (!data.version) dbs.delete(data.name);
      } else {
        for (const [n, s] of this._snapshot.stores) data.stores.set(n, s);
      }
    }
    _abort(error) {
      if (this._state === "finished" || this._state === "aborted") return;
      this._state = "aborted";
      this.error = error || null;
      this._rollback();
      // Requests still waiting fail with AbortError.
      for (const { req } of this._reqs.splice(0)) {
        req.readyState = "done";
        req._error = err("AbortError", "The transaction was aborted.");
        fire(req, "error", { bubbles: true, cancelable: true }, [this, this.db]);
      }
      fire(this, "abort", { bubbles: true }, [this.db]);
      this._done();
    }
    _done() {
      if (this._onDone) this._onDone();
      finished(this);
    }
    abort() {
      if (this._state === "finished" || this._state === "aborted") throw err("InvalidStateError", "The transaction has finished.");
      const go = () => this._abort(null);
      if (running === this) go(); else { const i = queue.indexOf(this); if (i >= 0) queue.splice(i, 1); this._state = "aborted"; this._rollback(); setTimeout(() => { fire(this, "abort", { bubbles: true }, [this.db]); }, 0); }
    }
    commit() {
      if (this._state !== "active") throw err("InvalidStateError", "The transaction has finished.");
      this._commitAsked = true;
    }
  }

  // ------------------------------------------------------------ object stores

  class IDBObjectStore {
    constructor(tx, name) { this.transaction = tx; this._name = name; this._indexes = new Map(); }
    get _s() {
      const s = this.transaction.db._data.stores.get(this._name);
      if (!s) throw err("InvalidStateError", "The object store has been deleted.");
      return s;
    }
    get name() { return this._name; }
    set name(n) {
      n = String(n);
      if (this.transaction.mode !== "versionchange") throw err("InvalidStateError", "Stores can only be renamed while upgrading.");
      if (n === this._name) return;
      const stores = this.transaction.db._data.stores;
      if (stores.has(n)) throw err("ConstraintError", "A store named " + n + " already exists.");
      this.transaction._touch();
      const s = this._s;
      stores.delete(this._name);
      s.name = n;
      stores.set(n, s);
      this.transaction._stores.delete(this._name);
      this.transaction._stores.set(n, this);
      this._name = n;
    }
    get keyPath() { const k = this._s.keyPath; return Array.isArray(k) ? k.slice() : k; }
    get autoIncrement() { return this._s.autoIncrement; }
    get indexNames() { return stringList(this._s.idx.keys()); }

    _req(op) { return this.transaction._add(new IDBRequest(this, this.transaction), op); }

    _put(value, key, noOverwrite) {
      const tx = this.transaction;
      this._s;
      tx._active();
      tx._write();
      const s = this._s;
      if (s.keyPath !== null && key !== undefined) throw err("DataError", "This store uses in-line keys; don't pass a key.");
      if (s.keyPath === null && !s.autoIncrement && key === undefined) throw err("DataError", "This store needs a key.");
      let k = key !== undefined ? mustKey(key) : undefined;
      const clone = decode(encode(value));
      let needKey = false;
      if (s.keyPath !== null) {
        const v = evalPath(clone, s.keyPath);
        if (v === undefined) {
          if (!s.autoIncrement) throw err("DataError", "The value has no key at the store's key path.");
          if (Array.isArray(s.keyPath) || !canInject(clone, s.keyPath)) throw err("DataError", "The key can't be put into the value.");
          needKey = true;
        } else {
          k = mustKey(v);
        }
      } else if (k === undefined) {
        needKey = true;
      }
      return this._req(() => {
        const s = this._s;
        tx._touch();
        if (needKey) {
          if (s.current > 9007199254740992) throw err("ConstraintError", "The key generator has run out of keys.");
          k = ["n", s.current++];
          if (s.keyPath !== null) injectKey(clone, s.keyPath, k[1]);
        } else if (s.autoIncrement && k[0] === "n" && k[1] >= s.current) {
          s.current = Math.floor(k[1]) + 1;
        }
        const at = lower(s.recs, k, (r) => r.k);
        const exists = at < s.recs.length && cmp(s.recs[at].k, k) === 0;
        if (exists && noOverwrite) throw err("ConstraintError", "A record with this key already exists.");
        for (const ix of s.idx.values()) {
          if (!ix.unique) continue;
          for (const ik of indexKeys(ix, clone)) {
            const i = lower(ix.ents, ik, (e) => e.k);
            if (i < ix.ents.length && cmp(ix.ents[i].k, ik) === 0 && cmp(ix.ents[i].p, k) !== 0) throw err("ConstraintError", "The index " + ix.name + " needs unique values.");
          }
        }
        const rec = { k, v: encode(clone) };
        if (exists) {
          s.recs[at] = rec;
          for (const ix of s.idx.values()) removeEntries(ix, k);
        } else {
          s.recs.splice(at, 0, rec);
        }
        for (const ix of s.idx.values()) addEntries(ix, k, clone);
        return keyValue(k);
      });
    }
    put(value, key) { return this._put(value, key, false); }
    add(value, key) { return this._put(value, key, true); }

    _read(query, required, op) {
      this._s; // throws InvalidStateError first if the store was deleted
      this.transaction._active();
      const range = toRange(query, required);
      return this._req(() => op(this._s, range));
    }
    get(query) {
      return this._read(query, true, (s, r) => { const [a, b] = span(s.recs, r, (x) => x.k); return a < b ? decode(s.recs[a].v) : undefined; });
    }
    getKey(query) {
      return this._read(query, true, (s, r) => { const [a, b] = span(s.recs, r, (x) => x.k); return a < b ? keyValue(s.recs[a].k) : undefined; });
    }
    // getAll / getAllKeys / getAllRecords: (query, count) or an options object
    // { query, count, direction }.
    _all(query, count, pick) {
      const o = allOptions(query, count);
      return this._read(o.query, false, (s, r) => {
        const [a, b] = span(s.recs, r, (x) => x.k);
        return takeAll(s.recs.slice(a, b), o, (x) => x.k, (x) => x.k).map((x) => pick(x.k, x.k, x.v));
      });
    }
    getAll(query, count) { return this._all(query, count, (k, p, v) => decode(v)); }
    getAllKeys(query, count) { return this._all(query, count, (k) => keyValue(k)); }
    getAllRecords(opts) { return this._all(opts || {}, undefined, record); }
    count(query) {
      return this._read(query, false, (s, r) => { const [a, b] = span(s.recs, r, (x) => x.k); return b - a; });
    }
    delete(query) {
      this._s;
      this.transaction._active();
      this.transaction._write();
      const range = toRange(query, true);
      return this._req(() => {
        const s = this._s;
        this.transaction._touch();
        const [a, b] = span(s.recs, range, (x) => x.k);
        const gone = s.recs.splice(a, b - a);
        for (const r of gone) for (const ix of s.idx.values()) removeEntries(ix, r.k);
        return undefined;
      });
    }
    clear() {
      this._s;
      this.transaction._active();
      this.transaction._write();
      return this._req(() => {
        const s = this._s;
        this.transaction._touch();
        s.recs = [];
        for (const ix of s.idx.values()) ix.ents = [];
        return undefined;
      });
    }
    openCursor(query, direction) { return openCursor(this, null, query, direction, false); }
    openKeyCursor(query, direction) { return openCursor(this, null, query, direction, true); }

    createIndex(name, keyPath, opts) {
      const tx = this.transaction;
      if (tx.mode !== "versionchange") throw err("InvalidStateError", "Indexes can only be created while upgrading.");
      tx._active();
      name = String(name);
      const s = this._s;
      if (s.idx.has(name)) throw err("ConstraintError", "An index named " + name + " already exists.");
      keyPath = checkKeyPath(keyPath);
      if (keyPath === null) throw err("SyntaxError", "An index needs a key path.");
      opts = opts || {};
      if (opts.multiEntry && Array.isArray(keyPath)) throw err("InvalidAccessError", "A multiEntry index can't use an array key path.");
      tx._touch();
      const ix = { name, keyPath, unique: !!opts.unique, multiEntry: !!opts.multiEntry, ents: [] };
      for (const r of s.recs) addEntries(ix, r.k, decode(r.v));
      s.idx.set(name, ix);
      if (ix.unique) {
        for (let i = 1; i < ix.ents.length; i++) {
          if (cmp(ix.ents[i - 1].k, ix.ents[i].k) === 0) {
            // Existing data breaks the constraint: the upgrade fails.
            setTimeout(() => tx._abort(err("ConstraintError", "Existing records break the unique index " + name + ".")), 0);
            break;
          }
        }
      }
      return this.index(name);
    }
    deleteIndex(name) {
      const tx = this.transaction;
      if (tx.mode !== "versionchange") throw err("InvalidStateError", "Indexes can only be deleted while upgrading.");
      if (!this._s.idx.has(name)) throw err("NotFoundError", "No index named " + name + ".");
      tx._touch();
      this._s.idx.delete(name);
      this._indexes.delete(name);
    }
    index(name) {
      if (!this._s.idx.has(name)) throw err("NotFoundError", "No index named " + name + ".");
      let i = this._indexes.get(name);
      if (!i) { i = new IDBIndex(this, name); this._indexes.set(name, i); }
      return i;
    }
  }

  // ------------------------------------------------------------ indexes

  class IDBIndex {
    constructor(store, name) { this.objectStore = store; this._name = name; }
    get _ix() {
      const ix = this.objectStore._s.idx.get(this._name);
      if (!ix) throw err("InvalidStateError", "The index has been deleted.");
      return ix;
    }
    get name() { return this._name; }
    get keyPath() { const k = this._ix.keyPath; return Array.isArray(k) ? k.slice() : k; }
    get unique() { return this._ix.unique; }
    get multiEntry() { return this._ix.multiEntry; }
    _read(query, required, op) {
      this._ix; // throws InvalidStateError first if the index or store was deleted
      const tx = this.objectStore.transaction;
      tx._active();
      const range = toRange(query, required);
      return tx._add(new IDBRequest(this, tx), () => op(this._ix, this.objectStore._s, range));
    }
    _value(s, p) {
      const i = lower(s.recs, p, (r) => r.k);
      return decode(s.recs[i].v);
    }
    get(query) {
      return this._read(query, true, (ix, s, r) => { const [a, b] = span(ix.ents, r, (e) => e.k); return a < b ? this._value(s, ix.ents[a].p) : undefined; });
    }
    getKey(query) {
      return this._read(query, true, (ix, s, r) => { const [a, b] = span(ix.ents, r, (e) => e.k); return a < b ? keyValue(ix.ents[a].p) : undefined; });
    }
    _all(query, count, pick) {
      const o = allOptions(query, count);
      return this._read(o.query, false, (ix, s, r) => {
        const [a, b] = span(ix.ents, r, (e) => e.k);
        return takeAll(ix.ents.slice(a, b), o, (e) => e.k, (e) => e.p).map((e) => pick(e.k, e.p, s.recs[lower(s.recs, e.p, (x) => x.k)].v));
      });
    }
    getAll(query, count) { return this._all(query, count, (k, p, v) => decode(v)); }
    getAllKeys(query, count) { return this._all(query, count, (k, p) => keyValue(p)); }
    getAllRecords(opts) { return this._all(opts || {}, undefined, record); }
    count(query) {
      return this._read(query, false, (ix, s, r) => { const [a, b] = span(ix.ents, r, (e) => e.k); return b - a; });
    }
    openCursor(query, direction) { return openCursor(this.objectStore, this, query, direction, false); }
    openKeyCursor(query, direction) { return openCursor(this.objectStore, this, query, direction, true); }
  }

  // ------------------------------------------------------------ getAll options

  class IDBRecord {
    constructor(k, p, v) { this._k = k; this._p = p; this._v = v; }
    get key() { return keyValue(this._k); }
    get primaryKey() { return keyValue(this._p); }
    get value() { return decode(this._v); }
  }
  const record = (k, p, v) => new IDBRecord(k, p, v);

  // (query, count) or { query, count, direction } -> { query, count, direction }.
  function allOptions(query, count) {
    let o = { query, count, direction: "next" };
    const plain = query && typeof query === "object" && !(query instanceof IDBKeyRange) && !Array.isArray(query) && !(query instanceof Date) && !(query instanceof ArrayBuffer) && !ArrayBuffer.isView(query);
    if (plain) {
      o = { query: query.query, count: query.count, direction: query.direction === undefined ? "next" : String(query.direction) };
      if (!DIRS.includes(o.direction)) throw new TypeError("Unknown direction: " + o.direction);
    }
    if (o.count !== undefined) {
      const n = Number(o.count);
      if (!Number.isFinite(n) || n < 0 || n > 4294967295) throw new TypeError("count must be a whole number from 0 to 2^32-1.");
      o.count = Math.floor(n);
    }
    return o;
  }

  // Apply direction (prev reverses; unique keeps the first entry of each key) and count.
  function takeAll(items, o, keyOfItem, primaryOfItem) {
    let out = items;
    if (o.direction.endsWith("unique")) {
      out = [];
      for (const it of items) if (!out.length || cmp(keyOfItem(out[out.length - 1]), keyOfItem(it)) !== 0) out.push(it);
    }
    if (o.direction.startsWith("prev")) {
      if (o.direction === "prevunique") {
        // prevunique lists keys backwards, each at its lowest primary key.
        out = out.slice().reverse();
      } else {
        out = out.slice().reverse();
      }
    }
    return o.count > 0 ? out.slice(0, o.count) : out;
  }

  // ------------------------------------------------------------ cursors

  const DIRS = ["next", "nextunique", "prev", "prevunique"];

  class IDBCursor {
    constructor(store, index, req, range, direction, keysOnly) {
      this._store = store; this._index = index; this.request = req;
      this._range = range; this._dir = direction; this._keysOnly = keysOnly;
      this._k = null; this._p = null; this._v = undefined;
      this._got = false;
    }
    get source() { return this._index || this._store; }
    get direction() { return this._dir; }
    get key() { return this._k === null ? undefined : keyValue(this._k); }
    get primaryKey() { return this._p === null ? undefined : keyValue(this._p); }
    // Find the next position after (key, primaryKey) — or the first, from the start.
    _advance(targetKey, targetPrimary) {
      const back = this.direction.startsWith("prev");
      const unique = this.direction.endsWith("unique");
      const s = this._store._s;
      const inRange = (k) => !this._range || this._range._has(k);
      if (!this._index) {
        const arr = s.recs;
        let i;
        if (!back) {
          i = this._k === null ? 0 : upper(arr, this._k, (r) => r.k);
          if (targetKey) i = Math.max(i, lower(arr, targetKey, (r) => r.k));
          if (this._range && this._range._l !== null) i = Math.max(i, this._range.lowerOpen ? upper(arr, this._range._l, (r) => r.k) : lower(arr, this._range._l, (r) => r.k));
        } else {
          i = (this._k === null ? arr.length : lower(arr, this._k, (r) => r.k)) - 1;
          if (targetKey) i = Math.min(i, upper(arr, targetKey, (r) => r.k) - 1);
          if (this._range && this._range._u !== null) i = Math.min(i, (this._range.upperOpen ? lower(arr, this._range._u, (r) => r.k) : upper(arr, this._range._u, (r) => r.k)) - 1);
        }
        if (i < 0 || i >= arr.length || !inRange(arr[i].k)) return false;
        this._k = arr[i].k; this._p = arr[i].k; this._v = arr[i].v;
        return true;
      }
      const ix = this._index._ix;
      const arr = ix.ents;
      const at = (i) => {
        this._k = arr[i].k; this._p = arr[i].p;
        const r = s.recs[lower(s.recs, arr[i].p, (x) => x.k)];
        this._v = r && r.v;
        return true;
      };
      if (!back) {
        let i;
        if (this._k === null) i = 0;
        else if (unique) i = upper(arr, this._k, (e) => e.k);
        else {
          const cur = { k: this._k, p: this._p };
          i = entLower(arr, cur);
          if (i < arr.length && entCmp(arr[i], cur) === 0) i++;
        }
        if (targetKey) {
          const j = targetPrimary ? entLower(arr, { k: targetKey, p: targetPrimary }) : lower(arr, targetKey, (e) => e.k);
          i = Math.max(i, j);
        }
        if (this._range && this._range._l !== null) i = Math.max(i, this._range.lowerOpen ? upper(arr, this._range._l, (e) => e.k) : lower(arr, this._range._l, (e) => e.k));
        if (i >= arr.length || !inRange(arr[i].k)) return false;
        return at(i);
      }
      let i;
      if (this._k === null) i = arr.length - 1;
      else if (unique) i = lower(arr, this._k, (e) => e.k) - 1;
      else i = entLower(arr, { k: this._k, p: this._p }) - 1;
      if (targetKey) {
        let j;
        if (targetPrimary) {
          // The last entry at or before (key, primaryKey).
          const t = { k: targetKey, p: targetPrimary };
          j = entLower(arr, t);
          if (!(j < arr.length && entCmp(arr[j], t) === 0)) j--;
        } else {
          j = upper(arr, targetKey, (e) => e.k) - 1;
        }
        i = Math.min(i, j);
      }
      if (this._range && this._range._u !== null) i = Math.min(i, (this._range.upperOpen ? lower(arr, this._range._u, (e) => e.k) : upper(arr, this._range._u, (e) => e.k)) - 1);
      if (i < 0 || !inRange(arr[i].k)) return false;
      // prevunique lands on the first entry of its key.
      if (unique) i = lower(arr, arr[i].k, (e) => e.k);
      return at(i);
    }
    _queue(targetKey, targetPrimary, count) {
      const tx = this._store.transaction;
      tx._active();
      if (!this._got) throw err("InvalidStateError", "The cursor is already moving or has finished.");
      this._got = false;
      const req = this.request;
      req.readyState = "pending";
      tx._reqs.push({
        req,
        op: () => {
          let n = count || 1;
          let ok = true;
          while (n-- > 0 && ok) ok = this._advance(n === 0 ? targetKey : null, n === 0 ? targetPrimary : null);
          if (!ok) { this._k = this._p = null; return null; }
          this._got = true;
          return this;
        },
      });
    }
    continue(key) {
      let t = null;
      if (key !== undefined) {
        t = mustKey(key);
        if (this._k !== null) {
          const c = cmp(t, this._k);
          if (this.direction.startsWith("prev") ? c >= 0 : c <= 0) throw err("DataError", "continue() must move the cursor forward.");
        }
      }
      this._queue(t, null);
    }
    continuePrimaryKey(key, primaryKey) {
      if (!this._index) throw err("InvalidAccessError", "continuePrimaryKey() works on index cursors.");
      if (this.direction.endsWith("unique")) throw err("InvalidAccessError", "continuePrimaryKey() doesn't work with unique directions.");
      this._queue(mustKey(key), mustKey(primaryKey));
    }
    advance(count) {
      count = Number(count);
      if (!(count > 0) || !Number.isInteger(count)) throw new TypeError("advance() needs a positive whole number.");
      this._queue(null, null, count);
    }
    update(value) {
      if (arguments.length === 0) throw new TypeError("update() needs a value.");
      const tx = this._store.transaction;
      tx._write();
      if (this._keysOnly || !this._got) throw err("InvalidStateError", "The cursor has no value here.");
      const s = this._store._s;
      if (s.keyPath !== null) {
        const v = evalPath(value, s.keyPath);
        const k = v === undefined ? undefined : keyOf(v);
        if (k === undefined || cmp(k, this._p) !== 0) throw err("DataError", "update() can't change the record's key.");
        return this._store._put(value, undefined, false);
      }
      return this._store._put(value, keyValue(this._p), false);
    }
    delete() {
      const tx = this._store.transaction;
      tx._write();
      if (this._keysOnly || !this._got) throw err("InvalidStateError", "The cursor has no value here.");
      return this._store.delete(keyValue(this._p));
    }
  }

  class IDBCursorWithValue extends IDBCursor {
    // The same object each time until the cursor moves (scripts edit it, then update()).
    get value() {
      if (this._v === undefined) return undefined;
      if (this._vFrom !== this._v) { this._vFrom = this._v; this._vObj = decode(this._v); }
      return this._vObj;
    }
  }

  function openCursor(store, index, query, direction, keysOnly) {
    if (index) index._ix; else store._s;
    store.transaction._active();
    direction = direction === undefined ? "next" : String(direction);
    if (!DIRS.includes(direction)) throw new TypeError("Unknown cursor direction: " + direction);
    const range = toRange(query, false);
    const req = new IDBRequest(index || store, store.transaction);
    const cursor = new (keysOnly ? IDBCursor : IDBCursorWithValue)(store, index, req, range, direction, keysOnly);
    return store.transaction._add(req, () => {
      if (!cursor._advance(null, null)) return null;
      cursor._got = true;
      return cursor;
    });
  }

  // ------------------------------------------------------------ databases

  const connections = new Set();

  class IDBDatabase extends EventTarget {
    constructor(data) {
      super();
      this._data = data;
      this.name = data.name;
      this._closed = false;
      this._upgrade = null;
      this.onversionchange = null;
      this.onclose = null;
      this.onabort = null;
      this.onerror = null;
      connections.add(this);
    }
    get version() { return this._version; }
    get objectStoreNames() { return stringList(this._data.stores.keys()); }
    createObjectStore(name, opts) {
      const tx = this._upgrade;
      if (!tx || tx._state === "finished" || tx._state === "aborted") throw err("InvalidStateError", "Object stores can only be created while upgrading.");
      tx._active();
      name = String(name);
      if (this._data.stores.has(name)) throw err("ConstraintError", "An object store named " + name + " already exists.");
      opts = opts || {};
      const keyPath = checkKeyPath(opts.keyPath);
      if (opts.autoIncrement && (keyPath === "" || Array.isArray(keyPath))) throw err("InvalidAccessError", "autoIncrement needs a simple key path or none.");
      tx._touch();
      this._data.stores.set(name, { name, keyPath, autoIncrement: !!opts.autoIncrement, current: 1, recs: [], idx: new Map() });
      return tx.objectStore(name);
    }
    deleteObjectStore(name) {
      const tx = this._upgrade;
      if (!tx || tx._state === "finished" || tx._state === "aborted") throw err("InvalidStateError", "Object stores can only be deleted while upgrading.");
      tx._active();
      if (!this._data.stores.has(name)) throw err("NotFoundError", "No object store named " + name + ".");
      tx._touch();
      this._data.stores.delete(name);
      tx._stores.delete(name);
    }
    transaction(names, mode, opts) {
      if (this._closed) throw err("InvalidStateError", "The database connection is closed.");
      if (this._upgrade && this._upgrade._state === "active") throw err("InvalidStateError", "An upgrade is still running.");
      names = typeof names === "string" ? [names] : Array.from(names || [], String);
      names = [...new Set(names)];
      if (!names.length) throw err("InvalidAccessError", "A transaction needs at least one object store.");
      for (const n of names) if (!this._data.stores.has(n)) throw err("NotFoundError", "No object store named " + n + ".");
      mode = mode === undefined ? "readonly" : mode;
      if (mode !== "readonly" && mode !== "readwrite") throw new TypeError("Unknown transaction mode: " + mode);
      const tx = new IDBTransaction(this, names, mode, opts && opts.durability);
      queue.push(tx);
      pump();
      return tx;
    }
    close() {
      this._closed = true;
      connections.delete(this);
    }
  }

  function openOthers(name, except) {
    return [...connections].filter((c) => c.name === name && c !== except && !c._closed);
  }

  // Ask other open connections to close; wait (firing "blocked" once) until they have.
  function whenClosed(name, except, req, oldVersion, newVersion, then) {
    for (const c of openOthers(name, except)) fire(c, "versionchange", { oldVersion, newVersion });
    let blocked = false;
    const check = () => {
      if (!openOthers(name, except).length) return then();
      if (!blocked) { blocked = true; fire(req, "blocked", { oldVersion, newVersion }); }
      setTimeout(check, 20);
    };
    check();
  }

  // An open or delete request waits its turn in the queue like a transaction.
  class Job {
    constructor(run) { this._run = run; }
    _start() { this._run(() => finished(this)); }
  }

  const indexedDB = {
    open(name, version) {
      if (name === undefined) throw new TypeError("open() needs a database name.");
      name = String(name);
      if (version !== undefined) {
        version = Number(version);
        if (!Number.isInteger(version) || version < 1 || version > 9007199254740991) throw new TypeError("The version must be a positive whole number.");
      }
      const req = new IDBOpenDBRequest();
      queue.push(new Job((done) => {
        const all = load();
        let data = all.get(name);
        const oldVersion = data ? data.version : 0;
        const want = version === undefined ? oldVersion || 1 : version;
        const fail = (e) => {
          req.readyState = "done";
          req._error = e;
          req._result = undefined;
          fire(req, "error", { bubbles: true, cancelable: true });
          done();
        };
        if (want < oldVersion) return fail(err("VersionError", "The database is already at version " + oldVersion + "."));
        const succeed = (db) => {
          req.readyState = "done";
          req._result = db;
          req._error = null;
          req.transaction = null;
          fire(req, "success");
          done();
        };
        if (want === oldVersion) {
          const db = new IDBDatabase(data);
          db._version = oldVersion;
          return succeed(db);
        }
        whenClosed(name, null, req, oldVersion, want, () => {
          if (!data) {
            data = { name, version: 0, stores: new Map() };
            all.set(name, data);
          }
          const db = new IDBDatabase(data);
          const tx = new IDBTransaction(db, [], "versionchange");
          tx._oldVersion = oldVersion;
          tx._touch();
          data.version = want;
          db._version = want;
          db._upgrade = tx;
          req.transaction = tx;
          req.readyState = "done";
          req._result = db;
          tx._onDone = () => {
            db._upgrade = null;
            req.transaction = null;
            if (tx._state === "aborted") {
              db._version = oldVersion;
              db.close();
              req._result = undefined;
              req._error = err("AbortError", "The upgrade was aborted.");
              fire(req, "error", { bubbles: true, cancelable: true });
              done();
            } else {
              succeed(db);
            }
          };
          running = tx; // the upgrade runs now, in place of this job
          const ev = fire(req, "upgradeneeded", { oldVersion, newVersion: want });
          if (ev.threw) tx._abort(err("AbortError", "The upgradeneeded handler threw."));
          else tx._start();
        });
      }));
      pump();
      return req;
    },
    deleteDatabase(name) {
      name = String(name);
      const req = new IDBOpenDBRequest();
      queue.push(new Job((done) => {
        const all = load();
        const data = all.get(name);
        const oldVersion = data ? data.version : 0;
        whenClosed(name, null, req, oldVersion, null, () => {
          if (data) { all.delete(name); save(); }
          req.readyState = "done";
          req._result = undefined;
          fire(req, "success", { oldVersion, newVersion: null });
          done();
        });
      }));
      pump();
      return req;
    },
    databases() {
      return Promise.resolve(Array.from(load().values(), (d) => ({ name: d.name, version: d.version })));
    },
    cmp(a, b) {
      return cmp(mustKey(a), mustKey(b));
    },
  };

  Object.assign(g, {
    indexedDB,
    IDBFactory: function IDBFactory() { throw new TypeError("Illegal constructor"); },
    IDBKeyRange,
    IDBRequest,
    IDBOpenDBRequest,
    IDBTransaction,
    IDBDatabase,
    IDBObjectStore,
    IDBIndex,
    IDBCursor,
    IDBCursorWithValue,
    IDBVersionChangeEvent,
    IDBRecord,
  });
  Object.setPrototypeOf(indexedDB, g.IDBFactory.prototype);
  for (const C of [g.IDBFactory, IDBKeyRange, IDBRequest, IDBOpenDBRequest, IDBTransaction, IDBDatabase, IDBObjectStore, IDBIndex, IDBCursor, IDBCursorWithValue, IDBVersionChangeEvent, IDBRecord]) {
    Object.defineProperty(C.prototype, Symbol.toStringTag, { value: C.name, configurable: true });
  }
  // The structured clone used for postMessage too, now that it handles more types.
  g.structuredClone = (v) => decode(encode(v));
})();
