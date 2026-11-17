// Constructable stylesheets: `new CSSStyleSheet()`, replace()/replaceSync(), cssRules,
// insertRule()/deleteRule(). Libraries such as Lit test `x instanceof CSSStyleSheet` while
// loading. Sheets aren't adopted into documents yet (no adoptedStyleSheets), so Lit and
// others fall back to <style> elements, which are drawn.
(function () {
  "use strict";
  const g = globalThis;
  if (typeof g.CSSStyleSheet === "function") return;

  // Split CSS text into top-level rules (`sel { … }` or `@rule …;` / `@rule { … }`).
  function splitRules(text) {
    const out = [];
    let depth = 0, start = 0, quote = null;
    const s = String(text).replace(/\/\*[\s\S]*?\*\//g, "");
    for (let i = 0; i < s.length; i++) {
      const c = s[i];
      if (quote) { if (c === "\\") i++; else if (c === quote) quote = null; continue; }
      if (c === '"' || c === "'") quote = c;
      else if (c === "{") depth++;
      else if (c === "}") { if (--depth === 0) { out.push(s.slice(start, i + 1).trim()); start = i + 1; } }
      else if (c === ";" && depth === 0) { const r = s.slice(start, i + 1).trim(); if (r.length > 1) out.push(r); start = i + 1; }
    }
    return out.filter(Boolean);
  }

  class CSSRule {
    constructor(text, sheet) {
      this.cssText = text;
      this.parentStyleSheet = sheet;
      this.parentRule = null;
      const m = /^([^{]*)\{/.exec(text);
      this.type = text.startsWith("@media") ? 4 : text.startsWith("@import") ? 3 : text.startsWith("@font-face") ? 5 : text.startsWith("@keyframes") ? 7 : text.startsWith("@") ? 0 : 1;
      if (this.type === 1 && m) this.selectorText = m[1].trim();
    }
  }
  CSSRule.STYLE_RULE = 1; CSSRule.IMPORT_RULE = 3; CSSRule.MEDIA_RULE = 4; CSSRule.FONT_FACE_RULE = 5; CSSRule.KEYFRAMES_RULE = 7;

  class StyleSheet {}
  class CSSStyleSheet extends StyleSheet {
    constructor(options = {}) {
      super();
      this.__rules = [];
      this.disabled = !!options.disabled;
      this.media = { mediaText: typeof options.media === "string" ? options.media : "" };
      this.href = null;
      this.ownerNode = null;
      this.ownerRule = null;
      this.parentStyleSheet = null;
      this.title = null;
      this.type = "text/css";
    }
    get cssRules() { return this.__rules.slice(); }
    get rules() { return this.cssRules; }
    replaceSync(text) {
      // @import isn't allowed in constructed sheets; such rules are dropped.
      this.__rules = splitRules(text).filter((r) => !/^@import\b/i.test(r)).map((r) => new CSSRule(r, this));
    }
    replace(text) {
      try { this.replaceSync(text); return Promise.resolve(this); } catch (e) { return Promise.reject(e); }
    }
    insertRule(rule, index = 0) {
      const parts = splitRules(rule);
      if (parts.length !== 1) throw new (g.DOMException || Error)("insertRule needs exactly one rule", "SyntaxError");
      if (index < 0 || index > this.__rules.length) throw new (g.DOMException || Error)("index out of range", "IndexSizeError");
      this.__rules.splice(index, 0, new CSSRule(parts[0], this));
      return index;
    }
    deleteRule(index) {
      if (index < 0 || index >= this.__rules.length) throw new (g.DOMException || Error)("index out of range", "IndexSizeError");
      this.__rules.splice(index, 1);
    }
    addRule(selector, style, index = this.__rules.length) { return this.insertRule(`${selector} { ${style} }`, index), -1; }
    removeRule(index = 0) { this.deleteRule(index); }
  }

  g.StyleSheet = StyleSheet;
  g.CSSStyleSheet = CSSStyleSheet;
  g.CSSRule = CSSRule;
  g.CSSStyleRule = CSSRule;
})();
