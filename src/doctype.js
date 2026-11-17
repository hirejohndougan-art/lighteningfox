// DocumentType and document.doctype (The Guardian's consent script, 2026-11: `DocumentType is
// not defined`). The doctype node is found among the document's children; its name and ids
// come from __doctype.
(function () {
  "use strict";
  const g = globalThis;
  const d = g.document;
  if (!d || g.DocumentType || typeof __doctype !== "function") return;
  class DocumentType extends g.Node {
    constructor() { throw new TypeError("Illegal constructor"); }
    static [Symbol.hasInstance](x) { return x != null && typeof x === "object" && x.nodeType === 10; }
    get [Symbol.toStringTag]() { return "DocumentType"; }
    get name() { const i = __doctype(this.__id); return i ? i[0] : ""; }
    get publicId() { const i = __doctype(this.__id); return i ? i[1] : ""; }
    get systemId() { const i = __doctype(this.__id); return i ? i[2] : ""; }
    get nodeName() { return this.name; }
    get textContent() { return null; }
    set textContent(v) {}
    remove() { if (this.parentNode) this.parentNode.removeChild(this); }
  }
  g.DocumentType = DocumentType;
  // Doctype nodes are made as plain nodes: give one DocumentType's members when it's reached.
  const upgrade = (n) => {
    if (n && n.nodeType === 10 && Object.getPrototypeOf(n) !== DocumentType.prototype) Object.setPrototypeOf(n, DocumentType.prototype);
    return n;
  };
  if (!("doctype" in d)) {
    Object.defineProperty(d, "doctype", {
      configurable: true,
      get() {
        for (const c of d.childNodes) if (c.nodeType === 10) return upgrade(c);
        return null;
      },
    });
  }
  // document.firstChild and childNodes[0] reach it too.
  const fc = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(d), "firstChild") || Object.getOwnPropertyDescriptor(g.Node.prototype, "firstChild");
  if (fc && fc.get) {
    Object.defineProperty(d, "firstChild", { configurable: true, get() { return upgrade(fc.get.call(this)); } });
  }
})();
