// TreeWalker and NodeIterator (DOM Standard §6): document.createTreeWalker and
// document.createNodeIterator. Lit and other template libraries walk their templates with a
// TreeWalker as soon as they load, so without these whole sites' scripts stop early.
(function () {
  "use strict";
  const g = globalThis;
  const ACCEPT = 1, REJECT = 2, SKIP = 3;

  g.NodeFilter = Object.assign(g.NodeFilter || {}, {
    FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3,
    SHOW_ALL: 0xffffffff, SHOW_ELEMENT: 0x1, SHOW_ATTRIBUTE: 0x2, SHOW_TEXT: 0x4, SHOW_CDATA_SECTION: 0x8,
    SHOW_ENTITY_REFERENCE: 0x10, SHOW_ENTITY: 0x20, SHOW_PROCESSING_INSTRUCTION: 0x40, SHOW_COMMENT: 0x80,
    SHOW_DOCUMENT: 0x100, SHOW_DOCUMENT_TYPE: 0x200, SHOW_DOCUMENT_FRAGMENT: 0x400, SHOW_NOTATION: 0x800,
  });

  // Runs the filter (§6.1 "filter"): whatToShow first, then the callback or acceptNode().
  function filter(t, node) {
    if (t.__active) throw new (g.DOMException || Error)("filter is already running", "InvalidStateError");
    const bit = 1 << ((node.nodeType || 1) - 1);
    if (!(t.whatToShow & bit)) return SKIP;
    const f = t.filter;
    if (f == null) return ACCEPT;
    t.__active = true;
    try {
      const r = typeof f === "function" ? f.call(undefined, node) : f.acceptNode(node);
      return Number(r) || 0;
    } finally {
      t.__active = false;
    }
  }

  const isInclusiveAncestor = (a, b) => { for (let n = b; n; n = n.parentNode) if (n === a) return true; return false; };

  class TreeWalker {
    constructor(root, whatToShow, filter) {
      Object.defineProperty(this, "root", { value: root, enumerable: true });
      Object.defineProperty(this, "whatToShow", { value: whatToShow >>> 0, enumerable: true });
      Object.defineProperty(this, "filter", { value: filter ?? null, enumerable: true });
      this.__current = root;
      this.__active = false;
    }
    get currentNode() { return this.__current; }
    set currentNode(n) {
      if (n == null || typeof n !== "object") throw new TypeError("currentNode must be a Node");
      this.__current = n;
    }
    parentNode() {
      let n = this.__current;
      while (n && n !== this.root) {
        n = n.parentNode;
        if (n && filter(this, n) === ACCEPT) return (this.__current = n);
      }
      return null;
    }
    __children(first) {
      let n = first ? this.__current.firstChild : this.__current.lastChild;
      while (n) {
        const r = filter(this, n);
        if (r === ACCEPT) return (this.__current = n);
        if (r === SKIP) {
          const c = first ? n.firstChild : n.lastChild;
          if (c) { n = c; continue; }
        }
        while (n) {
          const s = first ? n.nextSibling : n.previousSibling;
          if (s) { n = s; break; }
          const p = n.parentNode;
          if (!p || p === this.root || p === this.__current) return null;
          n = p;
        }
      }
      return null;
    }
    firstChild() { return this.__children(true); }
    lastChild() { return this.__children(false); }
    __siblings(next) {
      let node = this.__current;
      if (node === this.root) return null;
      for (;;) {
        let s = next ? node.nextSibling : node.previousSibling;
        while (s) {
          node = s;
          const r = filter(this, node);
          if (r === ACCEPT) return (this.__current = node);
          s = next ? node.firstChild : node.lastChild;
          if (r === REJECT || !s) s = next ? node.nextSibling : node.previousSibling;
        }
        node = node.parentNode;
        if (!node || node === this.root) return null;
        if (filter(this, node) === ACCEPT) return null;
      }
    }
    nextSibling() { return this.__siblings(true); }
    previousSibling() { return this.__siblings(false); }
    previousNode() {
      let node = this.__current;
      while (node !== this.root) {
        let s = node.previousSibling;
        while (s) {
          node = s;
          let r = filter(this, node);
          while (r !== REJECT && node.lastChild) {
            node = node.lastChild;
            r = filter(this, node);
          }
          if (r === ACCEPT) return (this.__current = node);
          s = node.previousSibling;
        }
        if (node === this.root || !node.parentNode) return null;
        node = node.parentNode;
        if (filter(this, node) === ACCEPT) return (this.__current = node);
      }
      return null;
    }
    nextNode() {
      let node = this.__current;
      let r = ACCEPT;
      for (;;) {
        while (r !== REJECT && node.firstChild) {
          node = node.firstChild;
          r = filter(this, node);
          if (r === ACCEPT) return (this.__current = node);
        }
        let s = null, t = node;
        while (t) {
          if (t === this.root) return null;
          s = t.nextSibling;
          if (s) break;
          t = t.parentNode;
        }
        if (!s) return null;
        node = s;
        r = filter(this, node);
        if (r === ACCEPT) return (this.__current = node);
      }
    }
  }

  class NodeIterator {
    constructor(root, whatToShow, filter) {
      Object.defineProperty(this, "root", { value: root, enumerable: true });
      Object.defineProperty(this, "whatToShow", { value: whatToShow >>> 0, enumerable: true });
      Object.defineProperty(this, "filter", { value: filter ?? null, enumerable: true });
      this.__ref = root;
      this.__before = true;
      this.__active = false;
    }
    get referenceNode() { return this.__ref; }
    get pointerBeforeReferenceNode() { return this.__before; }
    __following(n) {
      if (n.firstChild) return n.firstChild;
      for (let t = n; t && t !== this.root; t = t.parentNode) if (t.nextSibling) return t.nextSibling;
      return null;
    }
    __preceding(n) {
      if (n === this.root) return null;
      let p = n.previousSibling;
      if (!p) return n.parentNode;
      while (p.lastChild) p = p.lastChild;
      return p;
    }
    __traverse(next) {
      let node = this.__ref, before = this.__before;
      for (;;) {
        if (next) {
          if (!before) { node = this.__following(node); if (!node) return null; } else before = false;
        } else {
          if (before) { node = this.__preceding(node); if (!node) return null; } else before = true;
        }
        if (filter(this, node) === ACCEPT) break;
      }
      this.__ref = node;
      this.__before = before;
      return node;
    }
    nextNode() { return this.__traverse(true); }
    previousNode() { return this.__traverse(false); }
    detach() {}
  }

  g.TreeWalker = TreeWalker;
  g.NodeIterator = NodeIterator;
  const make = (Cls) => function (root, whatToShow = 0xffffffff, filter = null) {
    if (root == null || typeof root !== "object" || !("nodeType" in root)) throw new TypeError("root must be a Node");
    return new Cls(root, whatToShow, filter);
  };
  const targets = [g.document, g.Document && g.Document.prototype].filter(Boolean);
  for (const t of targets) {
    if (typeof t.createTreeWalker !== "function") Object.defineProperty(t, "createTreeWalker", { value: make(TreeWalker), configurable: true, writable: true });
    if (typeof t.createNodeIterator !== "function") Object.defineProperty(t, "createNodeIterator", { value: make(NodeIterator), configurable: true, writable: true });
  }
})();
