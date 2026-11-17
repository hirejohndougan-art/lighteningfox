// How the developer console shows a value: strings quoted, elements as <tag id class>,
// arrays and objects as a short preview (like other browsers' consoles).
(function () {
  "use strict";
  const g = globalThis;
  function show(v, depth, seen) {
    if (v === undefined) return "undefined";
    if (v === null) return "null";
    const t = typeof v;
    if (t === "string") return depth ? JSON.stringify(v) : JSON.stringify(v);
    if (t === "number" || t === "boolean" || t === "bigint") return String(v) + (t === "bigint" ? "n" : "");
    if (t === "symbol") return v.toString();
    if (t === "function") return "ƒ " + (v.name || "anonymous") + "()";
    if (seen.has(v)) return "[circular]";
    if (v instanceof Error) return (v.name || "Error") + ": " + v.message;
    if (v && v.nodeType === 1) {
      let s = "<" + v.localName;
      if (v.id) s += " id=\"" + v.id + "\"";
      if (v.className) s += " class=\"" + v.className + "\"";
      return s + ">";
    }
    if (v && v.nodeType === 3) return "#text " + JSON.stringify(v.textContent);
    if (v === g.document) return "#document";
    if (v === g) return "Window";
    if (typeof Promise !== "undefined" && v instanceof Promise) return "Promise";
    if (depth > 1) return Array.isArray(v) ? "Array(" + v.length + ")" : "{…}";
    seen.add(v);
    if (Array.isArray(v)) {
      const items = v.slice(0, 20).map((x) => show(x, depth + 1, seen));
      if (v.length > 20) items.push("… " + (v.length - 20) + " more");
      return "[" + items.join(", ") + "]";
    }
    if (v instanceof Map) return "Map(" + v.size + ")";
    if (v instanceof Set) return "Set(" + v.size + ")";
    const keys = Object.keys(v).filter((k) => !k.startsWith("__"));
    const parts = keys.slice(0, 20).map((k) => k + ": " + show(v[k], depth + 1, seen));
    if (keys.length > 20) parts.push("…");
    const name = v.constructor && v.constructor !== Object && v.constructor.name ? v.constructor.name + " " : "";
    return name + "{" + parts.join(", ") + "}";
  }
  g.__inspect = (v) => { try { return show(v, 0, new Set()); } catch (e) { return String(v); } };
})();
