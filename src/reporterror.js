// self.reportError(e): report an exception like an uncaught one without throwing it — the
// window's onerror (message, source, line, column, error) and "error" listeners, then the
// console unless one of them handled it. React reports recoverable errors (hydration
// mismatches) through it; NYT wraps it and calls the saved original, which threw
// "cannot convert 'null' or 'undefined' to object" where it was missing.
(function () {
  "use strict";
  const g = globalThis;
  if (typeof g.reportError === "function") return;
  function reportError(e) {
    if (arguments.length < 1) throw new TypeError("Failed to execute 'reportError' on 'Window': 1 argument required, but only 0 present.");
    const message = e instanceof Error ? `${e.name}: ${e.message}` : String(e);
    let handled = false;
    // onerror gets the five-argument form; returning true marks the error handled.
    const onerror = g.onerror;
    if (typeof onerror === "function") {
      try { if (onerror.call(g, "Uncaught " + message, "", 0, 0, e) === true) handled = true; } catch (x) { console.error(x); }
    }
    const EE = g.ErrorEvent;
    if (EE && typeof g.dispatchEvent === "function") {
      const ev = new EE("error", { message: "Uncaught " + message, filename: "", lineno: 0, colno: 0, error: e, cancelable: true });
      // The handler property was already called above.
      g.onerror = null;
      try { if (!g.dispatchEvent(ev) || ev.defaultPrevented) handled = true; } finally { g.onerror = onerror; }
    }
    if (!handled) console.error(e);
  }
  Object.defineProperty(g, "reportError", { value: reportError, writable: true, configurable: true, enumerable: true });
})();
