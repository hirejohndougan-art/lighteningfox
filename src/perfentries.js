// Entry types the browser records itself (blob.js has marks and measures):
// - "navigation": the entry docprops.js makes is a PerformanceNavigationTiming, as pages check
//   with instanceof (The Guardian decides whether the page "performs poorly" that way, and
//   without the class waited 2.4 s for a first paint before starting its scripts).
// - "paint": first-paint and first-contentful-paint, recorded after the first layout that
//   drew something, for PerformanceObserver({type: "paint", buffered: true}) and
//   getEntriesByType("paint").
(function () {
  const g = globalThis, perf = g.performance;
  if (!perf || !g.PerformanceEntry || !perf.__record) return;
  class PerformanceNavigationTiming extends g.PerformanceEntry {}
  class PerformanceResourceTiming extends g.PerformanceEntry {}
  Object.setPrototypeOf(PerformanceNavigationTiming.prototype, PerformanceResourceTiming.prototype);
  class PerformancePaintTiming extends g.PerformanceEntry {}
  const nav = perf.getEntriesByType("navigation")[0];
  if (nav && !(nav instanceof PerformanceNavigationTiming)) Object.setPrototypeOf(nav, PerformanceNavigationTiming.prototype);
  Object.assign(g, { PerformanceNavigationTiming, PerformanceResourceTiming, PerformancePaintTiming });
  let painted = false;
  (g.__layoutHooks ||= []).push(() => {
    if (painted || !g.document || !g.document.body) return;
    painted = true;
    const t = perf.now();
    for (const name of ["first-paint", "first-contentful-paint"]) perf.__record(new PerformancePaintTiming(name, "paint", t, 0));
  });
})();
