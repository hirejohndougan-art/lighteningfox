// The WebExtensions API (`chrome.*` / `browser.*`) for one extension, as content scripts and
// background scripts see it. Rust answers through one native, __extCall(json) -> json
// ({"ok": result} or {"error": message}), and delivers events with api.__dispatch(event).
(function () {
  "use strict";
  const g = globalThis;
  if (g.__extMakeApi) return;

  function makeEvent() {
    const listeners = [];
    return {
      _l: listeners,
      addListener(f) { if (typeof f === "function" && !listeners.includes(f)) listeners.push(f); },
      removeListener(f) { const i = listeners.indexOf(f); if (i >= 0) listeners.splice(i, 1); },
      hasListener(f) { return listeners.includes(f); },
      hasListeners() { return listeners.length > 0; },
      _fire(...args) {
        for (const f of listeners.slice()) {
          try { f(...args); } catch (e) { console.error(e); }
        }
      },
    };
  }

  g.__extMakeApi = function (id, kind, manifest) {
    const background = kind === "background";
    const runtime = { id, lastError: undefined };
    const raw = (req) => {
      req.ext = id;
      let r;
      try { r = JSON.parse(__extCall(JSON.stringify(req))); } catch (e) { r = { error: String(e) }; }
      return r;
    };
    // Call Rust and return a promise; a trailing callback gets the result too (with
    // runtime.lastError set while it runs, if it failed).
    const call = (req, cb, map) => {
      const r = raw(req);
      const value = r.error === undefined ? (map ? map(r.ok) : r.ok) : undefined;
      if (typeof cb === "function") {
        Promise.resolve().then(() => {
          runtime.lastError = r.error === undefined ? undefined : { message: r.error };
          try { cb(value); } finally { runtime.lastError = undefined; }
        });
        return Promise.resolve(value);
      }
      return r.error === undefined ? Promise.resolve(value) : Promise.reject(new Error(r.error));
    };
    const lastCb = (args) => (typeof args[args.length - 1] === "function" ? args.pop() : undefined);

    // ---- runtime
    runtime.onMessage = makeEvent();
    runtime.onInstalled = makeEvent();
    runtime.onStartup = makeEvent();
    runtime.onConnect = makeEvent();
    runtime.getManifest = () => JSON.parse(JSON.stringify(manifest));
    runtime.getURL = (p) => "ironswift-extension://" + id + "/" + String(p).replace(/^\/+/, "");
    runtime.getPlatformInfo = (cb) => call({ op: "noop" }, cb, () => ({ os: "linux", arch: "x86-64", nacl_arch: "x86-64" }));
    // Replies to tabs.sendMessage (background) and runtime.sendMessage (content) waiting here.
    const waiting = new Map();
    runtime.sendMessage = function (...args) {
      const cb = lastCb(args);
      // (extensionId, message, options) or (message, options) or (message)
      let msg = args.length >= 2 && typeof args[0] === "string" && args[0] !== id && args.length === 3 ? args[1] : args[0];
      if (args.length === 2 && typeof args[0] === "string" && args[0] === id) msg = args[1];
      if (background) {
        // No other extension pages here: nobody hears it.
        return call({ op: "noop" }, cb, () => undefined);
      }
      return call({ op: "send", msg: msg === undefined ? null : JSON.parse(JSON.stringify(msg)) }, cb, (r) => (r && "response" in r ? r.response : undefined));
    };

    // Run onMessage listeners for a message; answer with `reply(response | undefined)`.
    function hear(msg, sender, reply) {
      let answered = false, async = false;
      const sendResponse = (r) => {
        if (answered) return;
        answered = true;
        reply(r === undefined ? undefined : JSON.parse(JSON.stringify(r)));
      };
      for (const f of runtime.onMessage._l.slice()) {
        let ret;
        try { ret = f(msg, sender, sendResponse); } catch (e) { console.error(e); continue; }
        if (ret === true) async = true;
        else if (ret && typeof ret.then === "function") {
          async = true;
          ret.then(sendResponse, (e) => { console.error(e); sendResponse(undefined); });
        }
      }
      if (!answered && !async) { answered = true; reply(undefined, true); }
    }

    // ---- storage
    const area = (name) => ({
      get(...a) { const cb = lastCb(a); return call({ op: "storage", area: name, method: "get", arg: a[0] === undefined ? null : a[0] }, cb); },
      set(items, cb) { return call({ op: "storage", area: name, method: "set", arg: items }, cb, () => undefined); },
      remove(keys, cb) { return call({ op: "storage", area: name, method: "remove", arg: keys }, cb, () => undefined); },
      clear(cb) { return call({ op: "storage", area: name, method: "clear", arg: null }, cb, () => undefined); },
      getBytesInUse(...a) { const cb = lastCb(a); return call({ op: "storage", area: name, method: "getBytesInUse", arg: null }, cb); },
      onChanged: makeEvent(),
    });
    const storage = { local: area("local"), sync: area("sync"), session: area("session"), managed: area("managed"), onChanged: makeEvent() };

    const api = { runtime, storage, extension: { getURL: runtime.getURL, inIncognitoContext: false } };
    api.i18n = {
      getMessage: (name) => "",
      getUILanguage: () => (g.navigator && navigator.language) || "en-US",
    };

    // ---- background-only APIs
    let wrListeners = [];
    if (background) {
      const tabs = {
        TAB_ID_NONE: -1,
        query: (q, cb) => call({ op: "tabs.query", query: q || {} }, cb),
        get: (tabId, cb) => call({ op: "tabs.get", tabId }, cb),
        getCurrent: (cb) => call({ op: "noop" }, cb, () => undefined),
        create: (p, cb) => call({ op: "tabs.create", props: p || {} }, cb),
        update: (...a) => {
          const cb = lastCb(a);
          const [tabId, props] = a.length >= 2 ? a : [null, a[0]];
          return call({ op: "tabs.update", tabId, props: props || {} }, cb);
        },
        remove: (ids, cb) => call({ op: "tabs.remove", tabIds: Array.isArray(ids) ? ids : [ids] }, cb, () => undefined),
        reload: (...a) => { const cb = lastCb(a); const t = typeof a[0] === "number" ? a[0] : null; return call({ op: "tabs.reload", tabId: t }, cb, () => undefined); },
        sendMessage(tabId, msg, ...rest) {
          const cb = lastCb(rest);
          const r = raw({ op: "tabs.sendMessage", tabId, msg: msg === undefined ? null : JSON.parse(JSON.stringify(msg)) });
          const p = r.error !== undefined ? Promise.reject(new Error(r.error)) : new Promise((res, rej) => waiting.set(r.ok, { res, rej }));
          if (typeof cb === "function") {
            p.then((v) => cb(v), (e) => {
              runtime.lastError = { message: e.message };
              try { cb(undefined); } finally { runtime.lastError = undefined; }
            });
            return Promise.resolve();
          }
          return p;
        },
        onCreated: makeEvent(), onUpdated: makeEvent(), onRemoved: makeEvent(), onActivated: makeEvent(),
      };
      api.tabs = tabs;
      const onBeforeRequest = makeEvent();
      onBeforeRequest.addListener = function (f, filter, extra) {
        if (typeof f !== "function") throw new TypeError("listener must be a function");
        if (!filter || !Array.isArray(filter.urls)) throw new TypeError("webRequest filter needs urls");
        const blocking = Array.isArray(extra) && extra.includes("blocking");
        const r = raw({ op: "webRequest.listen", filter: { urls: filter.urls.map(String), types: filter.types || null }, blocking });
        if (r.error !== undefined) throw new Error(r.error);
        wrListeners.push({ f, index: r.ok, blocking });
      };
      onBeforeRequest.removeListener = function (f) {
        for (const l of wrListeners.filter((l) => l.f === f)) raw({ op: "webRequest.unlisten", index: l.index });
        wrListeners = wrListeners.filter((l) => l.f !== f);
      };
      onBeforeRequest.hasListener = (f) => wrListeners.some((l) => l.f === f);
      onBeforeRequest.hasListeners = () => wrListeners.length > 0;
      const later = () => makeEvent();
      api.webRequest = {
        onBeforeRequest,
        onBeforeSendHeaders: later(), onSendHeaders: later(), onHeadersReceived: later(),
        onResponseStarted: later(), onCompleted: later(), onErrorOccurred: later(), onBeforeRedirect: later(),
        handlerBehaviorChanged: (cb) => call({ op: "noop" }, cb, () => undefined),
      };
      api.browserAction = api.action = {
        onClicked: makeEvent(),
        setBadgeText: (d, cb) => call({ op: "noop" }, cb, () => undefined),
        setBadgeBackgroundColor: (d, cb) => call({ op: "noop" }, cb, () => undefined),
        setIcon: (d, cb) => call({ op: "noop" }, cb, () => undefined),
        setTitle: (d, cb) => call({ op: "noop" }, cb, () => undefined),
      };
    }

    // Events from Rust.
    api.__dispatch = function (ev) {
      switch (ev.type) {
        case "message": {
          // To a content script (from tabs.sendMessage) or the background (runtime.sendMessage).
          hear(ev.msg, ev.sender || { id }, (response, none) => raw({ op: "reply", id: ev.id, response: none ? null : { response } }));
          break;
        }
        case "reply": {
          const w = waiting.get(ev.id);
          if (!w) break;
          waiting.delete(ev.id);
          if (ev.error) w.rej(new Error(ev.error)); else w.res(ev.response);
          break;
        }
        case "storage": {
          storage.onChanged._fire(ev.changes, ev.area);
          if (storage[ev.area]) storage[ev.area].onChanged._fire(ev.changes);
          break;
        }
        case "tabs": {
          const e = api.tabs && api.tabs[ev.name];
          if (e) e._fire(...ev.args);
          break;
        }
        case "installed": {
          runtime.onInstalled._fire({ reason: "install" });
          runtime.onStartup._fire();
          break;
        }
        case "webRequest": {
          // Blocking listeners decide; the first cancel or redirect wins.
          let answer = {};
          const pending = [];
          for (const l of wrListeners) {
            if (!ev.listeners.includes(l.index)) continue;
            let r;
            try { r = l.f(Object.assign({}, ev.details)); } catch (e) { console.error(e); continue; }
            if (!l.blocking) continue;
            if (r && typeof r.then === "function") pending.push(r);
            else if (r && (r.cancel || r.redirectUrl) && !answer.cancel && !answer.redirectUrl) answer = r;
          }
          const send = (a) => raw({ op: "webRequest.reply", id: ev.id, response: { cancel: !!a.cancel, redirectUrl: a.redirectUrl ? String(a.redirectUrl) : null } });
          if (pending.length) {
            Promise.all(pending.map((p) => p.then((x) => x, () => null))).then((rs) => {
              for (const r of rs) if (r && (r.cancel || r.redirectUrl) && !answer.cancel && !answer.redirectUrl) answer = r;
              send(answer);
            });
          } else send(answer);
          break;
        }
      }
    };
    return api;
  };
})();
