// Inside a Web Worker: the global object becomes a DedicatedWorkerGlobalScope. Document-only
// things the shared prelude set up are removed, and the worker's own globals are added.
(function () {
  "use strict";
  const g = globalThis;
  const gone = ["document", "window", "localStorage", "sessionStorage", "history", "alert", "confirm", "prompt",
    "open", "print", "frames", "parent", "top", "opener", "frameElement", "customElements", "getComputedStyle",
    "matchMedia", "scrollTo", "scrollBy", "scroll", "innerWidth", "innerHeight", "outerWidth", "outerHeight",
    "scrollX", "scrollY", "pageXOffset", "pageYOffset", "screen", "devicePixelRatio", "requestAnimationFrame",
    "cancelAnimationFrame", "getSelection", "visualViewport", "Notification", "indexedDB", "IDBFactory",
    "MutationObserver", "IntersectionObserver", "ResizeObserver", "Image", "Option", "Audio", "focus", "blur", "stop"];
  for (const k of gone) {
    try { delete g[k]; } catch (e) {}
    if (k in g) { try { Object.defineProperty(g, k, { value: undefined, configurable: true, writable: true }); } catch (e) {} }
  }
  // Worker globals: a class so `self instanceof WorkerGlobalScope` works for the global.
  const scope = (name) => {
    const C = { [name]: function () { throw new TypeError("Illegal constructor"); } }[name];
    Object.defineProperty(C, Symbol.hasInstance, { value: (x) => x === g });
    return C;
  };
  g.WorkerGlobalScope = scope("WorkerGlobalScope");
  g.DedicatedWorkerGlobalScope = scope("DedicatedWorkerGlobalScope");
  try { Object.defineProperty(g, Symbol.toStringTag, { value: "DedicatedWorkerGlobalScope", configurable: true }); } catch (e) {}
  g.self = g;
  g.name = "";
  g.onmessage = g.onmessageerror = g.onerror = null;
  const toPage = (s) => __wPost(s);
  g.postMessage = function postMessage(data, transfer) {
    __wPost(g.__packOut(data, g.__transferList(transfer), toPage));
  };
  g.close = function close() { __wClose(); };
  g.importScripts = function importScripts(...urls) {
    for (const u of urls) {
      const why = __wImport(__resolve(g.location.href, String(u)));
      if (why) throw new g.DOMException("importScripts failed: " + why, "NetworkError");
    }
  };
  // A message from the page.
  g.__wMessage = function (s) { g.__arrive(s, toPage, g); };
  // Uncaught errors in handlers go to the Worker object's onerror on the page.
  // The worker's own onerror (and "error" listeners) see it first; unless one of them
  // handles it, it goes on to the Worker object's onerror on the page.
  g.__onListenerError = (e) => {
    const message = "Uncaught " + (e && e.message !== undefined ? (e.name ? e.name + ": " : "") + e.message : String(e));
    const filename = g.location.href;
    let handled = false;
    if (typeof g.onerror === "function") {
      try { handled = g.onerror(message, filename, 0, 0, e) === true; } catch (x) {}
    }
    const ev = new g.ErrorEvent("error", { message, filename, error: e, cancelable: true });
    const list = g.__listeners && g.__listeners.error;
    if (list) for (const l of list.slice()) { try { typeof l.fn === "function" ? l.fn.call(g, ev) : l.fn.handleEvent(ev); } catch (x) {} }
    if (!handled && !ev.defaultPrevented) __wError(message);
  };
  // WorkerNavigator and WorkerLocation are the window's minus the document parts.
  if (g.navigator) {
    for (const k of ["clipboard", "geolocation", "mediaDevices", "permissions"]) {
      if (k === "permissions") continue;
      try { Object.defineProperty(g.navigator, k, { value: undefined, configurable: true }); } catch (e) {}
    }
  }
})();
