// Namespaces: the …AttributeNS methods, getElementsByTagNameNS, and lookupNamespaceURI,
// lookupPrefix and isDefaultNamespace. Elements keep their namespace and prefix natively
// (createElementNS); attributes set with setAttributeNS keep theirs (__markAttrNS).
(() => {
  const g = globalThis;
  const XML = "http://www.w3.org/XML/1998/namespace", XMLNS = "http://www.w3.org/2000/xmlns/";
  const EP = g.Element.prototype, NP = g.Node.prototype;
  const nsArg = (v) => (v === null || v === undefined || String(v) === "" ? null : String(v));
  const check = (ns, q, attr) => {
    const r = __qname(nsArg(ns), String(q), attr);
    if (typeof r === "string") throw new g.DOMException("'" + q + "' is not a valid qualified name here.", r);
    return r;
  };
  const localOf = (q, n) => (n === null ? q : q.slice(q.indexOf(":") + 1));
  // The attribute with this namespace and local name: [qualifiedName, namespace, value].
  const findNS = (el, ns, local) => __attrsNS(el.__id).find(([q, n]) => n === ns && localOf(q, n) === local);

  const methods = {
    getAttributeNS(ns, local) { const a = findNS(this, nsArg(ns), String(local)); return a ? a[2] : null; },
    hasAttributeNS(ns, local) { return !!findNS(this, nsArg(ns), String(local)); },
    setAttributeNS(ns, q, v) {
      const [n, prefix, local] = check(ns, q, true);
      const old = findNS(this, n, local);
      const name = old ? old[0] : prefix === null ? local : prefix + ":" + local;
      // A plain setAttribute would lowercase the name on HTML elements: write it as given.
      __setAttr(this.__id, name, String(v));
      __markAttrNS(this.__id, name, n);
    },
    removeAttributeNS(ns, local) {
      const a = findNS(this, nsArg(ns), String(local));
      if (a) __removeAttr(this.__id, a[0]);
    },
    getElementsByTagNameNS(ns, local) {
      return __wrapList(__byTagNS(this.__id, ns === "*" ? "*" : nsArg(ns), String(local)));
    },
  };
  for (const [k, value] of Object.entries(methods)) {
    Object.defineProperty(EP, k, { value, writable: true, enumerable: false, configurable: true });
  }
  Object.defineProperty(g.Document.prototype, "getElementsByTagNameNS", { value: methods.getElementsByTagNameNS, writable: true, configurable: true });
  if (g.DocumentFragment) Object.defineProperty(g.DocumentFragment.prototype, "getElementsByTagNameNS", { value: methods.getElementsByTagNameNS, writable: true, configurable: true });

  // `attributes` entries carry their namespace parts too.
  const attrsDesc = Object.getOwnPropertyDescriptor(EP, "attributes");
  if (attrsDesc && attrsDesc.get) {
    Object.defineProperty(EP, "attributes", {
      get() {
        return __attrsNS(this.__id).map(([name, ns, value]) => {
          const i = ns === null ? -1 : name.indexOf(":");
          return { name, value, namespaceURI: ns, prefix: i < 0 ? null : name.slice(0, i), localName: i < 0 ? name : name.slice(i + 1), ownerElement: this, specified: true };
        });
      },
      enumerable: false,
      configurable: true,
    });
  }

  // DOM "locate a namespace" and "locate a namespace prefix".
  const elementOf = (node) => {
    switch (node.nodeType) {
      case 1: return node;
      case 9: return node.documentElement;
      case 10: case 11: return null;
      default: { const p = node.parentNode; return p && p.nodeType === 1 ? p : null; }
    }
  };
  function locateNamespace(node, prefix) {
    let el = elementOf(node);
    if (node.nodeType !== 1) return el ? locateNamespace(el, prefix) : null;
    if (prefix === "xml") return XML;
    if (prefix === "xmlns") return XMLNS;
    for (; el; el = el.parentNode && el.parentNode.nodeType === 1 ? el.parentNode : null) {
      if (el.namespaceURI !== null && el.prefix === prefix) return el.namespaceURI;
      for (const [q, n, v] of __attrsNS(el.__id)) {
        if (n !== XMLNS) continue;
        const i = q.indexOf(":"), p = i < 0 ? null : q.slice(0, i), l = i < 0 ? q : q.slice(i + 1);
        if ((p === "xmlns" && l === prefix) || (prefix === null && p === null && l === "xmlns")) return v === "" ? null : v;
      }
    }
    return null;
  }
  function locatePrefix(el, ns) {
    for (; el; el = el.parentNode && el.parentNode.nodeType === 1 ? el.parentNode : null) {
      if (el.namespaceURI === ns && el.prefix !== null) return el.prefix;
      for (const [q, n, v] of __attrsNS(el.__id)) {
        const i = q.indexOf(":");
        if (n === XMLNS && i >= 0 && q.slice(0, i) === "xmlns" && v === ns) return q.slice(i + 1);
      }
    }
    return null;
  }
  const lookup = {
    lookupNamespaceURI(prefix) {
      prefix = prefix === null || prefix === undefined || String(prefix) === "" ? null : String(prefix);
      return locateNamespace(this, prefix);
    },
    lookupPrefix(ns) {
      ns = nsArg(ns);
      if (ns === null) return null;
      const el = elementOf(this);
      return el ? locatePrefix(el, ns) : null;
    },
    isDefaultNamespace(ns) { return nsArg(ns) === locateNamespace(this, null); },
  };
  for (const [k, value] of Object.entries(lookup)) {
    Object.defineProperty(NP, k, { value, writable: true, enumerable: false, configurable: true });
    for (const C of [g.Element, g.Document, g.DocumentFragment, g.Text, g.Comment]) {
      if (C && C.prototype !== NP && Object.prototype.hasOwnProperty.call(C.prototype, k)) delete C.prototype[k];
    }
  }
})();
