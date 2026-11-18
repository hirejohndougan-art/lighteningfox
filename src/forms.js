// Form validation (validity, checkValidity, reportValidity, setCustomValidity) and file
// inputs (input.files), on top of the natives in validation.rs.
(function () {
  const g = globalThis;
  const E = g.Element.prototype;
  const KEY = Symbol("validity");
  const FLAGS = ["valueMissing", "typeMismatch", "patternMismatch", "tooLong", "tooShort", "rangeUnderflow", "rangeOverflow", "stepMismatch", "badInput", "customError"];
  const LISTED = new Set(["INPUT", "SELECT", "TEXTAREA", "BUTTON", "FIELDSET", "OUTPUT", "OBJECT"]);
  const isListed = (el) => el && LISTED.has(el.tagName);

  class ValidityState {
    constructor(key, id) {
      if (key !== KEY) throw new TypeError("Illegal constructor");
      Object.defineProperty(this, "_id", { value: id });
    }
    get valid() { const v = __validity(this._id); return !v || !v.slice(0, 10).some(Boolean); }
  }
  FLAGS.forEach((f, i) => Object.defineProperty(ValidityState.prototype, f, {
    get() { const v = __validity(this._id); return !!(v && v[i]); }, configurable: true, enumerable: true,
  }));
  Object.defineProperty(ValidityState.prototype, Symbol.toStringTag, { value: "ValidityState", configurable: true });
  g.ValidityState = ValidityState;

  const states = new WeakMap();
  const fire = (el) => el.dispatchEvent(new Event("invalid", { cancelable: true }));
  // The form's controls that are checked, in document order.
  const formControls = (form) => Array.from(document.querySelectorAll("input,select,textarea,button")).filter((e) => e.form === form);
  const invalid = (el) => { const v = __validity(el.__id); return !!v && v[11] && v.slice(0, 10).some(Boolean); };

  function check(el, report) {
    if (el.tagName === "FORM") {
      let first = null;
      let ok = true;
      for (const c of formControls(el)) {
        if (!invalid(c)) continue;
        ok = false;
        if (fire(c) && !first) first = c;
      }
      if (report && first) { __reportValidity(first.__id); first.focus(); }
      return ok;
    }
    if (el.tagName === "FIELDSET") return Array.from(el.querySelectorAll("input,select,textarea,button")).every((c) => check(c, false));
    if (!invalid(el)) return true;
    if (fire(el) && report) { __reportValidity(el.__id); el.focus(); }
    return false;
  }

  const define = (name, desc) => Object.defineProperty(E, name, { configurable: true, ...desc });
  define("validity", {
    get() {
      if (!isListed(this)) return undefined;
      let s = states.get(this);
      if (!s) states.set(this, (s = new ValidityState(KEY, this.__id)));
      return s;
    },
  });
  define("willValidate", { get() { const v = isListed(this) && __validity(this.__id); return !!(v && v[11]); } });
  define("validationMessage", { get() { const v = isListed(this) && __validity(this.__id); return v && v[11] ? v[10] : ""; } });
  define("checkValidity", { value: function checkValidity() { return check(this, false); }, writable: true });
  define("reportValidity", { value: function reportValidity() { return check(this, true); }, writable: true });
  define("setCustomValidity", { value: function setCustomValidity(msg) { __setCustomValidity(this.__id, String(msg)); }, writable: true });
  define("noValidate", { get() { return this.hasAttribute("novalidate"); }, set(v) { this.toggleAttribute("novalidate", !!v); } });
  define("formNoValidate", { get() { return this.hasAttribute("formnovalidate"); }, set(v) { this.toggleAttribute("formnovalidate", !!v); } });
  define("required", { get() { return this.hasAttribute("required"); }, set(v) { this.toggleAttribute("required", !!v); } });
  define("readOnly", { get() { return this.hasAttribute("readonly"); }, set(v) { this.toggleAttribute("readonly", !!v); } });
  define("multiple", { get() { return this.hasAttribute("multiple"); }, set(v) { this.toggleAttribute("multiple", !!v); } });
  for (const [prop, attr] of [["pattern", "pattern"], ["min", "min"], ["max", "max"], ["step", "step"], ["accept", "accept"]]) {
    define(prop, { get() { return this.getAttribute(attr) || ""; }, set(v) { this.setAttribute(attr, v); } });
  }
  for (const [prop, attr] of [["minLength", "minlength"], ["maxLength", "maxlength"]]) {
    define(prop, {
      get() { const n = parseInt(this.getAttribute(attr), 10); return n >= 0 ? n : -1; },
      set(v) { if (v < 0) throw new DOMException("The length can't be negative.", "IndexSizeError"); this.setAttribute(attr, String(v)); },
    });
  }

  // The interactive check the browser runs before submitting: fires `invalid` at each bad
  // control, and returns the one to point at (-1 = all fine, -2 = bad but every event cancelled).
  g.__interactiveValidate = function (formId) {
    const form = wrapId(formId);
    if (!form) return -1;
    let bad = false;
    let first = -2;
    for (const c of formControls(form)) {
      if (!invalid(c)) continue;
      bad = true;
      if (fire(c) && first < 0) first = c.__id;
    }
    return bad ? first : -1;
  };
  const wrapId = (id) => { const all = document.querySelectorAll("form"); for (const f of all) if (f.__id === id) return f; return null; };

  // ------------------------------------------------------------------ file inputs
  const fileLists = new WeakMap();
  class FileList {
    constructor(key, files) {
      if (key !== KEY) throw new TypeError("Illegal constructor");
      files.forEach((f, i) => { this[i] = f; });
      Object.defineProperty(this, "length", { value: files.length });
    }
    item(i) { return this[i] || null; }
    *[Symbol.iterator]() { for (let i = 0; i < this.length; i++) yield this[i]; }
  }
  Object.defineProperty(FileList.prototype, Symbol.toStringTag, { value: "FileList", configurable: true });
  g.FileList = FileList;
  const isFile = (el) => el.tagName === "INPUT" && (el.getAttribute("type") || "").toLowerCase() === "file";
  define("files", {
    get() {
      if (!isFile(this)) return null;
      const meta = __files(this.__id);
      const key = JSON.stringify(meta);
      const cached = fileLists.get(this);
      if (cached && cached.key === key) return cached.list;
      const list = new FileList(KEY, meta.map(([name, , type, modified], i) => new File([__fileBytes(this.__id, i)], name, { type, lastModified: modified })));
      fileLists.set(this, { key, list });
      return list;
    },
    set(v) { /* only the user picks files */ },
  });
  const valueDesc = Object.getOwnPropertyDescriptor(E, "value");
  if (valueDesc) {
    define("value", {
      get() {
        if (isFile(this)) { const f = __files(this.__id); return f.length ? "C:\\fakepath\\" + f[0][0] : ""; }
        return valueDesc.get.call(this);
      },
      set(v) {
        if (isFile(this)) {
          if (String(v) !== "") throw new DOMException("A file input's value can only be cleared.", "InvalidStateError");
          return;
        }
        valueDesc.set.call(this, v);
      },
    });
  }
  // ------------------------------------------------------------------ click() activation
  // el.click() on a checkbox or radio button checks it (undone if the click is cancelled);
  // on a submit or reset button it submits or resets the form.
  define("click", {
    writable: true,
    value: function click() {
      if (isListed(this) && this.disabled) return;
      const type = this.tagName === "INPUT" ? (this.getAttribute("type") || "text").toLowerCase() : this.tagName === "BUTTON" ? (this.getAttribute("type") || "submit").toLowerCase() : "";
      const toggles = this.tagName === "INPUT" && (type === "checkbox" || type === "radio");
      let undo = null;
      if (toggles) {
        const before = this.checked;
        const group = type === "radio" && this.name ? Array.from(document.querySelectorAll("input")).filter((r) => r !== this && (r.getAttribute("type") || "").toLowerCase() === "radio" && r.name === this.name && r.form === this.form && r.checked) : [];
        group.forEach((r) => { r.checked = false; });
        this.checked = type === "checkbox" ? !before : true;
        undo = () => { this.checked = before; group.forEach((r) => { r.checked = true; }); };
        if (this.checked === before && !group.length) undo = null;
      }
      const ev = typeof MouseEvent === "function" ? new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }) : new Event("click", { bubbles: true, cancelable: true, composed: true });
      const ok = this.dispatchEvent(ev);
      if (toggles) {
        if (!ok && undo) undo();
        else if (undo) {
          this.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
          this.dispatchEvent(new Event("change", { bubbles: true }));
        }
        return;
      }
      if (!ok) return;
      const form = this.form;
      if (form && (type === "submit" || type === "image")) form.requestSubmit(this.tagName === "BUTTON" || type === "submit" ? this : undefined);
      else if (form && type === "reset") form.reset();
    },
  });
  // ------------------------------------------------------------------ contenteditable
  define("contentEditable", {
    get() {
      const v = this.getAttribute("contenteditable");
      if (v === null) return "inherit";
      const l = v.trim().toLowerCase();
      return l === "" || l === "true" ? "true" : l === "false" ? "false" : l === "plaintext-only" ? "plaintext-only" : "inherit";
    },
    set(v) {
      const l = String(v).toLowerCase();
      if (l === "inherit") this.removeAttribute("contenteditable");
      else if (l === "true" || l === "false" || l === "plaintext-only") this.setAttribute("contenteditable", l);
      else throw new DOMException("Not a valid contentEditable value.", "SyntaxError");
    },
  });
  define("isContentEditable", {
    get() {
      for (let n = this; n && n.nodeType === 1; n = n.parentNode) {
        const v = n.getAttribute("contenteditable");
        if (v === null) continue;
        const l = v.trim().toLowerCase();
        if (l === "false") return false;
        if (l === "" || l === "true" || l === "plaintext-only") return true;
      }
      return false;
    },
  });
  // The browser's typing in a contenteditable host: beforeinput (cancellable) and input.
  g.__editEvent = function (id, type, inputType, data) {
    let el = null;
    for (const e of document.querySelectorAll("[contenteditable]")) if (e.__id === id) { el = e; break; }
    if (!el) return 1;
    const C = typeof InputEvent === "function" ? InputEvent : Event;
    const ev = new C(type, { bubbles: true, cancelable: type === "beforeinput", composed: true, inputType, data });
    if (C === Event) { Object.defineProperty(ev, "inputType", { value: inputType }); Object.defineProperty(ev, "data", { value: data }); }
    return el.dispatchEvent(ev) ? 1 : 0;
  };
})();
