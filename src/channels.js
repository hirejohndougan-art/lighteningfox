// Web Workers, WebSockets, the clipboard and blob: URLs. The Rust side (src/worker.rs,
// src/websocket.rs, src/clipboard.rs) does the threads, sockets and system clipboard;
// results come back through __workerEvent and __wsEvent when the page's tasks run.
(function () {
  "use strict";
  const g = globalThis;
  const Ev = g.Event;
  const DOMEx = (msg, name) => new g.DOMException(msg, name);
  const inWorker = !!g.__isWorker;

  // ------------------------------------------------------------ blob: URLs
  const blobUrls = new Map();
  g.__blobUrlStore = blobUrls;
  const uuid = () => "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === "x" ? r : (r & 3) | 8).toString(16);
  });
  if (typeof g.URL === "function") {
    try {
      g.URL.createObjectURL = function (obj) {
        const ms = g.__mseUrlFor && g.__mseUrlFor(obj);
        if (ms) return ms;
        if (!(obj instanceof g.Blob)) throw new TypeError("createObjectURL needs a Blob");
        const o = g.location && g.location.origin && g.location.origin !== "null" ? g.location.origin : "null";
        const u = "blob:" + o + "/" + uuid();
        blobUrls.set(u, obj);
        return u;
      };
      g.URL.revokeObjectURL = function (u) { blobUrls.delete(String(u)); if (g.__mseRevoke) g.__mseRevoke(u); };
    } catch (e) {}
  }
  const blobText = (b) => {
    const d = b.__bytes();
    return typeof g.TextDecoder === "function" ? new g.TextDecoder().decode(d) : Array.from(d, (c) => String.fromCharCode(c)).join("");
  };
  // fetch() of a blob: URL made on this page.
  if (typeof g.fetch === "function") {
    const netFetch = g.fetch;
    g.fetch = function (input, init) {
      const u = typeof input === "string" ? input : input && input.url;
      if (typeof u === "string" && u.startsWith("blob:")) {
        const b = blobUrls.get(u.split("#")[0]);
        if (!b) return Promise.reject(new TypeError("Failed to fetch"));
        return Promise.resolve(new g.Response(blobText(b), { status: 200, statusText: "OK", headers: { "content-type": b.type, "content-length": String(b.size) }, url: u }));
      }
      return netFetch.apply(this, arguments);
    };
  }

  const ErrorEvent = g.ErrorEvent || class ErrorEvent extends Ev {
    constructor(type, init) {
      super(type, init); init = init || {};
      this.message = init.message || ""; this.filename = init.filename || ""; this.lineno = init.lineno || 0; this.colno = init.colno || 0; this.error = init.error;
    }
  };
  g.ErrorEvent = ErrorEvent;
  const CloseEvent = g.CloseEvent || class CloseEvent extends Ev {
    constructor(type, init) { super(type, init); init = init || {}; this.code = init.code || 0; this.reason = init.reason || ""; this.wasClean = !!init.wasClean; }
  };
  g.CloseEvent = CloseEvent;

  // ------------------------------------------------------------ ports across threads
  // A MessagePort sent to (or from) a worker: its partner stays here and forwards through
  // the worker link; the other side gets a new port that forwards back. Messages travel as
  // envelopes: {"m": clone tree, "ports": [ids], "to": port id (for port messages)}.
  const MP = g.MessagePort;
  const remotePorts = new Map(); // port id -> the local port that receives its messages
  const newPid = () => Math.random().toString(36).slice(2) + "-" + (++pidSeq);
  let pidSeq = 0;
  function packOut(data, transfer, send, to) {
    const list = transfer ? Array.from(transfer) : [];
    const ids = new Map();
    for (const p of list) {
      if (!(p instanceof MP)) continue;
      if (p._shipped || ids.has(p)) throw DOMEx("A MessagePort could not be cloned because it was already transferred.", "DataCloneError");
      ids.set(p, newPid());
    }
    const tree = g.__cloneOut(data, list.filter((t) => !(t instanceof MP)), ids);
    for (const [p, pid] of ids) {
      p._shipped = true; p._closed = true;
      const partner = p._other;
      if (partner && !partner.__remote) { partner._other = { __remote: true, pid, send }; remotePorts.set(pid, partner); }
      else if (partner && partner.__remote) {
        // Already forwarding elsewhere: messages for it now go to the new holder.
        remotePorts.set(pid, null);
      }
    }
    return "{" + (to ? '"to":' + JSON.stringify(to) + "," : "") + '"ports":' + JSON.stringify([...ids.values()]) + ',"m":' + tree + "}";
  }
  function unpackIn(s, send) {
    const env = JSON.parse(s);
    const byId = {};
    const ports = [];
    for (const pid of env.ports || []) {
      const r = new MP();
      r._other = { __remote: true, pid, send };
      remotePorts.set(pid, r);
      byId[pid] = r;
      ports.push(r);
    }
    return { data: g.__cloneTree(env.m, byId), ports, to: env.to };
  }
  // A message that arrived over a link: for a forwarded port, or for the link's own target.
  function arrive(s, send, target) {
    let u;
    try { u = unpackIn(s, send); } catch (e) { target.dispatchEvent(new g.MessageEvent("messageerror", {})); return; }
    const dest = u.to ? remotePorts.get(u.to) : target;
    if (!dest || dest._closed) return;
    const ev = new g.MessageEvent("message", { data: u.data, ports: Object.freeze(u.ports) });
    if (dest === g) g.EventTarget.prototype.__fire.call(g, ev); else dest.dispatchEvent(ev);
  }
  g.__packOut = packOut;
  g.__arrive = arrive;
  if (MP) {
    const localPost = MP.prototype.postMessage;
    MP.prototype.postMessage = function (data, transfer) {
      const o = this._other;
      if (o && o.__remote) {
        const list = g.__transferList(transfer);
        if (list && Array.from(list).includes(this)) throw DOMEx("A port can't transfer itself.", "DataCloneError");
        if (this._closed) return;
        o.send(packOut(data, list, o.send, o.pid));
        return;
      }
      return localPost.call(this, data);
    };
  }

  // ------------------------------------------------------------ Worker
  const workers = new Map();
  let workerSeq = 0;
  class Worker extends g.EventTarget {
    constructor(url, opts) {
      super();
      if (arguments.length === 0) throw new TypeError("Worker needs a script URL");
      opts = opts || {};
      if (opts.type === "module") throw new TypeError("Module workers aren't supported yet");
      this.onmessage = this.onerror = this.onmessageerror = null;
      const abs = __resolve(g.location.href, String(url));
      let src = null;
      if (abs.startsWith("blob:")) {
        const b = blobUrls.get(abs.split("#")[0]);
        if (!b) throw DOMEx("The worker script at '" + abs + "' doesn't exist", "SecurityError");
        src = blobText(b);
      } else if (!abs.startsWith("data:")) {
        const same = (a, b) => { try { return new g.URL(a).origin === new g.URL(b).origin; } catch (e) { return false; } };
        if (!same(abs, g.location.href) && !g.location.href.startsWith("file:")) {
          throw DOMEx("Script at '" + abs + "' cannot be accessed from origin '" + g.location.origin + "'.", "SecurityError");
        }
      }
      const id = ++workerSeq;
      Object.defineProperty(this, "__wid", { value: id });
      workers.set(id, this);
      if (!__workerStart(id, abs, src, String(opts.name || ""))) {
        throw DOMEx("The worker couldn't be started", "SecurityError");
      }
    }
    postMessage(data, transfer) {
      const id = this.__wid;
      const s = packOut(data, g.__transferList(transfer), (x) => __workerPost(id, x));
      if (workers.has(id)) __workerPost(id, s);
    }
    terminate() {
      if (workers.delete(this.__wid)) __workerTerminate(this.__wid);
    }
  }
  g.Worker = Worker;
  const tag = (C, name) => Object.defineProperty(C.prototype, Symbol.toStringTag, { value: name, configurable: true });
  // From Rust: kind is "message", "error" or "exit".
  g.__workerEvent = function (id, kind, a, b, c) {
    const w = workers.get(id);
    if (!w) return;
    if (kind === "message") {
      arrive(a, (x) => __workerPost(id, x), w);
    } else if (kind === "error") {
      const ev = new ErrorEvent("error", { message: a, filename: b || "", lineno: c || 0, cancelable: true });
      w.dispatchEvent(ev);
      if (!ev.defaultPrevented) console.error("Uncaught error in worker: " + a);
    } else if (kind === "loaderror") {
      w.dispatchEvent(new Ev("error", { cancelable: true }));
    } else if (kind === "exit") {
      workers.delete(id);
    }
  };

  // ------------------------------------------------------------ WebSocket
  const sockets = new Map();
  let wsSeq = 0;
  const tokenRe = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
  class WebSocket extends g.EventTarget {
    constructor(url, protocols) {
      super();
      if (arguments.length === 0) throw new TypeError("WebSocket needs a URL");
      let u;
      try { u = new g.URL(String(url), g.location.href); } catch (e) { throw DOMEx("The URL '" + url + "' is invalid.", "SyntaxError"); }
      if (u.protocol === "http:") u.protocol = "ws:";
      else if (u.protocol === "https:") u.protocol = "wss:";
      if (u.protocol !== "ws:" && u.protocol !== "wss:") throw DOMEx("The URL's scheme must be either 'ws' or 'wss'. '" + u.protocol + "' is not allowed.", "SyntaxError");
      if (u.hash) throw DOMEx("The URL contains a fragment identifier ('" + u.hash + "'). Fragment identifiers are not allowed in WebSocket URLs.", "SyntaxError");
      if (protocols === undefined) protocols = [];
      else if (typeof protocols === "string") protocols = [protocols];
      else protocols = Array.from(protocols, String);
      const seen = new Set();
      for (const p of protocols) {
        if (!tokenRe.test(p) || seen.has(p)) throw DOMEx("The subprotocol '" + p + "' is invalid.", "SyntaxError");
        seen.add(p);
      }
      if (u.protocol === "ws:" && g.location.protocol === "https:" && !/^(localhost|127\.0\.0\.1|\[::1\])$/.test(u.hostname)) {
        throw DOMEx("An insecure WebSocket connection may not be initiated from a page loaded over HTTPS.", "SecurityError");
      }
      const id = ++wsSeq;
      Object.defineProperty(this, "__ws", { value: id });
      this._url = u.href; this._state = 0; this._protocol = ""; this._extensions = ""; this._binaryType = "blob"; this._buffered = 0;
      this.onopen = this.onmessage = this.onerror = this.onclose = null;
      sockets.set(id, this);
      __wsOpen(id, this._url, protocols.join(", "));
    }
    get url() { return this._url; }
    get readyState() { return this._state; }
    get protocol() { return this._protocol; }
    get extensions() { return this._extensions; }
    get bufferedAmount() { return this._state === 1 ? __wsBuffered(this.__ws) : this._buffered; }
    get binaryType() { return this._binaryType; }
    set binaryType(t) { if (t === "blob" || t === "arraybuffer") this._binaryType = t; }
    send(data) {
      if (this._state === 0) throw DOMEx("Still in CONNECTING state.", "InvalidStateError");
      if (typeof data === "string" || !(data instanceof ArrayBuffer || ArrayBuffer.isView(data) || (g.Blob && data instanceof g.Blob))) {
        const s = String(data);
        if (this._state !== 1) { this._buffered += s.length; return; }
        __wsSend(this.__ws, s, false);
        return;
      }
      let u8;
      if (data instanceof ArrayBuffer) u8 = new Uint8Array(data);
      else if (ArrayBuffer.isView(data)) u8 = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
      else u8 = data.__bytes();
      if (this._state !== 1) { this._buffered += u8.length; return; }
      let bin = "";
      for (let i = 0; i < u8.length; i += 0x8000) bin += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
      __wsSend(this.__ws, g.btoa(bin), true);
    }
    close(code, reason) {
      if (code !== undefined) {
        code = Number(code);
        if (code !== 1000 && !(code >= 3000 && code <= 4999)) throw DOMEx("The code must be either 1000, or between 3000 and 4999. " + code + " is neither.", "InvalidAccessError");
      }
      reason = reason === undefined ? "" : String(reason);
      if (new g.TextEncoder().encode(reason).length > 123) throw DOMEx("The message must not be greater than 123 bytes.", "SyntaxError");
      if (this._state >= 2) return;
      this._state = 2;
      __wsClose(this.__ws, code === undefined ? -1 : code, reason);
    }
  }
  for (const [k, v] of [["CONNECTING", 0], ["OPEN", 1], ["CLOSING", 2], ["CLOSED", 3]]) {
    Object.defineProperty(WebSocket, k, { value: v, enumerable: true });
    Object.defineProperty(WebSocket.prototype, k, { value: v, enumerable: true });
  }
  g.WebSocket = WebSocket;
  tag(Worker, "Worker"); tag(WebSocket, "WebSocket"); tag(ErrorEvent, "ErrorEvent"); tag(CloseEvent, "CloseEvent");
  g.__wsEvent = function (id, kind, a, b, c) {
    const ws = sockets.get(id);
    if (!ws) return;
    if (kind === "open") {
      ws._state = 1; ws._protocol = a || ""; ws._extensions = b || "";
      ws.dispatchEvent(new Ev("open"));
    } else if (kind === "text" || kind === "binary") {
      if (ws._state !== 1) return;
      let data = a;
      if (kind === "binary") {
        const s = g.atob(a);
        const u = new Uint8Array(s.length);
        for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
        data = ws._binaryType === "arraybuffer" ? u.buffer : new g.Blob([u]);
      }
      ws.dispatchEvent(new g.MessageEvent("message", { data, origin: new g.URL(ws._url).origin }));
    } else if (kind === "error") {
      ws.dispatchEvent(new Ev("error"));
    } else if (kind === "closing") {
      if (ws._state < 2) ws._state = 2;
    } else if (kind === "close") {
      sockets.delete(id);
      ws._state = 3;
      ws.dispatchEvent(new CloseEvent("close", { code: a, reason: b || "", wasClean: !!c }));
    }
  };

  if (inWorker) return;

  // ------------------------------------------------------------ clipboard
  // Writing needs a click or key press (a page can't fill your clipboard on its own);
  // reading asks you first, like location and notifications.
  class ClipboardItem {
    constructor(items, opts) {
      if (!items || typeof items !== "object") throw new TypeError("ClipboardItem needs items");
      this._items = Object.assign({}, items);
      this.presentationStyle = (opts && opts.presentationStyle) || "unspecified";
    }
    get types() { return Object.keys(this._items); }
    getType(type) {
      if (!(type in this._items)) return Promise.reject(DOMEx("The type '" + type + "' was not found", "NotFoundError"));
      return Promise.resolve(this._items[type]).then((v) => (v instanceof g.Blob ? v : new g.Blob([String(v)], { type })));
    }
    static supports(type) { return type === "text/plain"; }
  }
  g.ClipboardItem = ClipboardItem;
  const notAllowed = (what) => DOMEx(what, "NotAllowedError");
  const readOk = () => g.__permAskPublic("clipboard-read");
  class Clipboard extends g.EventTarget {
    writeText(text) {
      if (!__clipWrite(String(text))) return Promise.reject(notAllowed("Write permission denied: copying needs a click or key press."));
      return Promise.resolve();
    }
    readText() {
      return readOk().then((ok) => {
        if (!ok) throw notAllowed("Read permission denied.");
        const t = __clipRead();
        if (t === null) throw notAllowed("The clipboard couldn't be read.");
        return t;
      });
    }
    write(items) {
      const item = items && items[0];
      if (!(item instanceof ClipboardItem)) return Promise.reject(new TypeError("write() needs ClipboardItems"));
      if (!item.types.includes("text/plain")) return Promise.reject(DOMEx("Only text can be copied", "NotAllowedError"));
      // Check for the click now, before waiting on the item's data.
      if (!__clipWrite(null)) return Promise.reject(notAllowed("Write permission denied: copying needs a click or key press."));
      return item.getType("text/plain").then((b) => b.text()).then((t) => { __clipWrite(t); });
    }
    read() {
      return this.readText().then((t) => [new ClipboardItem({ "text/plain": new g.Blob([t], { type: "text/plain" }) })]);
    }
  }
  const clipboard = new Clipboard();
  Object.defineProperty(g.navigator, "clipboard", { get: () => clipboard, configurable: true });
  g.Clipboard = Clipboard;
  tag(Clipboard, "Clipboard"); tag(ClipboardItem, "ClipboardItem");
  // navigator.permissions.query knows the clipboard names too.
  const perms = g.navigator.permissions;
  if (perms && typeof perms.query === "function") {
    const q = perms.query.bind(perms);
    perms.query = function (desc) {
      const name = desc && desc.name;
      if (name === "clipboard-write") return Promise.resolve(Object.assign(new g.EventTarget(), { name, state: "granted", onchange: null }));
      if (name === "clipboard-read") {
        const st = Object.assign(new g.EventTarget(), { name, onchange: null });
        Object.defineProperty(st, "state", { get: () => __permState("clipboard-read") });
        return Promise.resolve(st);
      }
      return q(desc);
    };
  }
})();
