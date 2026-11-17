// ChildNode's before(), after() and replaceWith() on text and comment nodes (they
// were only on elements). From the New York Times (2026-11): Svelte places its blocks with
// anchor.before(fragment) on comment and text anchors, so the page's interactive parts and
// the elections widget failed to hydrate with "not a callable function". Follows the DOM
// spec: arguments are nodes or strings (made into text nodes), placed as one fragment next to
// the nearest sibling that isn't itself being moved.
(function () {
  "use strict";
  const g = globalThis;
  const d = g.document;
  if (!d || typeof g.CharacterData !== "function") return;

  const fragmentOf = (args) => {
    if (args.length === 1 && typeof args[0] === "object" && args[0] !== null) return args[0];
    const f = d.createDocumentFragment();
    for (const a of args) f.appendChild(typeof a === "object" && a !== null ? a : d.createTextNode(String(a)));
    return f;
  };
  const moving = (args) => new Set(args.filter((a) => typeof a === "object" && a !== null));

  const methods = {
    before(...nodes) {
      const parent = this.parentNode;
      if (!parent) return;
      const set = moving(nodes);
      let prev = this.previousSibling;
      while (prev && set.has(prev)) prev = prev.previousSibling;
      const node = fragmentOf(nodes);
      parent.insertBefore(node, prev ? prev.nextSibling : parent.firstChild);
    },
    after(...nodes) {
      const parent = this.parentNode;
      if (!parent) return;
      const set = moving(nodes);
      let next = this.nextSibling;
      while (next && set.has(next)) next = next.nextSibling;
      parent.insertBefore(fragmentOf(nodes), next);
    },
    replaceWith(...nodes) {
      const parent = this.parentNode;
      if (!parent) return;
      const set = moving(nodes);
      let next = this.nextSibling;
      while (next && set.has(next)) next = next.nextSibling;
      const node = fragmentOf(nodes);
      if (this.parentNode === parent) parent.replaceChild(node, this);
      else parent.insertBefore(node, next);
    },
  };
  const protos = [g.CharacterData.prototype];
  if (typeof g.DocumentType === "function") protos.push(g.DocumentType.prototype);
  for (const P of protos) {
    for (const [name, fn] of Object.entries(methods)) {
      if (typeof P[name] !== "function") Object.defineProperty(P, name, { value: fn, writable: true, configurable: true, enumerable: false });
    }
  }
})();
