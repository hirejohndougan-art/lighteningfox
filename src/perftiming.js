// Navigation timing (the legacy performance.timing and performance.navigation, and
// performance.timeOrigin): times in ms since the epoch, filled in as the page loads. Pages read
// these to report load speed; missing ones made their load handlers throw.
(function () {
  const g = globalThis;
  const perf = g.performance;
  if (!perf) return;
  const origin = Date.now() - (typeof perf.now === "function" ? perf.now() : 0);
  if (!("timeOrigin" in perf)) perf.timeOrigin = origin;
  const t = {};
  for (const k of ["navigationStart", "fetchStart", "domainLookupStart", "domainLookupEnd", "connectStart", "connectEnd", "requestStart", "responseStart", "responseEnd", "domLoading"]) t[k] = Math.round(origin);
  for (const k of ["unloadEventStart", "unloadEventEnd", "redirectStart", "redirectEnd", "secureConnectionStart", "domInteractive", "domContentLoadedEventStart", "domContentLoadedEventEnd", "domComplete", "loadEventStart", "loadEventEnd"]) t[k] = 0;
  const timing = perf.timing && typeof perf.timing === "object" ? Object.assign(perf.timing, t) : t;
  timing.toJSON = function () { const o = {}; for (const k of Object.keys(this)) if (k !== "toJSON") o[k] = this[k]; return o; };
  perf.timing = timing;
  if (!perf.navigation) perf.navigation = { type: 0, redirectCount: 0, TYPE_NAVIGATE: 0, TYPE_RELOAD: 1, TYPE_BACK_FORWARD: 2, TYPE_RESERVED: 255, toJSON() { return { type: 0, redirectCount: 0 }; } };
  const now = () => Math.round(origin + perf.now());
  if (g.document && g.document.addEventListener) {
    g.document.addEventListener("DOMContentLoaded", () => {
      timing.domInteractive = timing.domContentLoadedEventStart = now();
      setTimeout(() => { timing.domContentLoadedEventEnd = now(); }, 0);
    });
  }
  if (g.addEventListener) {
    g.addEventListener("load", () => {
      timing.domComplete = timing.loadEventStart = now();
      setTimeout(() => { timing.loadEventEnd = now(); }, 0);
    });
  }
})();
