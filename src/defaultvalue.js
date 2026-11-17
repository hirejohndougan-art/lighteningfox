// `defaultValue` that can be set (BBC News, 2026-11: React sets `input.defaultValue = …` while
// hydrating a form, and in strict mode a getter with no setter throws, which stopped the page).
// The `value` attribute (the text, for <textarea>). Added only where no setter exists yet.
(function () {
  "use strict";
  const g = globalThis;
  const proto = g.Element && g.Element.prototype;
  if (!proto) return;
  let p = proto, desc;
  for (; p && !desc; p = Object.getPrototypeOf(p)) desc = Object.getOwnPropertyDescriptor(p, "defaultValue");
  if (desc && desc.set) return;
  Object.defineProperty(proto, "defaultValue", {
    get() { return this.tagName === "TEXTAREA" ? this.textContent : this.getAttribute("value") || ""; },
    set(v) { if (this.tagName === "TEXTAREA") this.textContent = String(v); else this.setAttribute("value", String(v)); },
    configurable: true,
  });
})();
