// Navigation timing as the page loads: blob.js sets up performance.timing and
// performance.navigation; this fills in the DOMContentLoaded and load timestamps when they happen
// (pages report load speed from them, and Bing's load handler reads them).
(function () {
  const g = globalThis;
  const perf = g.performance;
  if (!perf) return;
  const timing = perf.timing || (perf.timing = {});
  const origin = typeof perf.timeOrigin === "number" ? perf.timeOrigin : Date.now() - perf.now();
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
