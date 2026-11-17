// The window's event handler properties (window.onload, onresize, onmessage…): they exist,
// null until set, so `onload = f` works in strict code, and dispatch on the window runs them
// (prelude's dispatch calls this["on" + type]). Elements' and documents' handler properties
// are read the same way, so documents get them too.
(function () {
  const g = globalThis;
  const names = ("abort afterprint animationend animationiteration animationstart beforeprint beforeunload blur canplay change click " +
    "contextmenu dblclick drag dragend dragenter dragleave dragover dragstart drop error focus focusin focusout gotpointercapture " +
    "hashchange input invalid keydown keypress keyup languagechange load loadeddata loadedmetadata loadstart lostpointercapture " +
    "message messageerror mousedown mouseenter mouseleave mousemove mouseout mouseover mouseup offline online pagehide pageshow " +
    "pause play playing pointercancel pointerdown pointerenter pointerleave pointermove pointerout pointerover pointerup popstate " +
    "progress rejectionhandled reset resize scroll scrollend select storage submit timeupdate toggle touchcancel touchend touchmove " +
    "touchstart transitionend unhandledrejection unload visibilitychange wheel").split(" ");
  for (const n of names) {
    const p = "on" + n;
    if (!(p in g)) {
      try { Object.defineProperty(g, p, { value: null, writable: true, configurable: true, enumerable: true }); } catch (e) {}
    }
  }
})();
