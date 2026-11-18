// The Text and Comment constructors (`new Text("x")`, `new Comment("x")`). The classes the
// prelude defines take a node id (they wrap existing nodes), so `new Text("x")` made an object
// that wasn't a node: DOM methods called on it threw "Illegal invocation: not a DOM node".
// From the New York Times (2026-11), whose Svelte widgets failed to hydrate around the same
// calls. The public constructors now make a new node and return its usual wrapper; the
// prototypes stay the same objects, so `instanceof` and anything added to Text.prototype
// keep working.
(function () {
  "use strict";
  const g = globalThis;
  const d = g.document;
  if (!d || typeof g.Text !== "function" || typeof g.Comment !== "function") return;

  const make = (Old, create, name) => {
    const ctor = function (data) {
      if (!new.target) throw new TypeError("Failed to construct '" + name + "': Please use the 'new' operator, this DOM object constructor cannot be called as a function.");
      return create(data === undefined ? "" : String(data));
    };
    Object.defineProperty(ctor, "name", { value: name });
    ctor.prototype = Old.prototype;
    Object.setPrototypeOf(ctor, Object.getPrototypeOf(Old));
    Object.defineProperty(Old.prototype, "constructor", { value: ctor, writable: true, configurable: true, enumerable: false });
    Object.defineProperty(g, name, { value: ctor, writable: true, configurable: true, enumerable: false });
  };
  make(g.Text, (s) => d.createTextNode(s), "Text");
  make(g.Comment, (s) => d.createComment(s), "Comment");
})();
