// <dialog> and popovers (dialog.rs keeps the top layer they're drawn in).
//
// - dialog.show() / showModal() / close(returnValue) / requestClose(), returnValue, `open`;
//   a modal dialog goes in the top layer over its ::backdrop, Esc cancels it, focus moves into
//   it and clicks and keys outside it don't reach the page; `<form method=dialog>` closes its
//   dialog with the submit button's value.
// - The popover attribute: showPopover() / hidePopover() / togglePopover(force),
//   beforetoggle and toggle events, popovertarget / popovertargetaction buttons, Esc and clicks
//   outside close "auto" popovers (light dismiss).
(function () {
  const g = globalThis;
  if (typeof HTMLElement !== "function" || g.__dialogInstalled) return;
  g.__dialogInstalled = true;
  // Elements' own prototype is Element's (the HTML interfaces only answer instanceof).
  const P = Element.prototype;
  const isDialog = (el) => el && el.localName === "dialog";
  // The top layer, bottom first: [element, "modal" | "popover"].
  const layer = [];
  const top = () => layer[layer.length - 1];
  const topModal = () => { for (let i = layer.length - 1; i >= 0; i--) if (layer[i][1] === "modal") return layer[i][0]; return null; };
  const addTop = (el, kind) => {
    const i = layer.findIndex((e) => e[0] === el);
    if (i >= 0) layer.splice(i, 1);
    layer.push([el, kind]);
    __topLayerAdd(el.__id, kind === "modal" ? 0 : 1);
  };
  const removeTop = (el) => {
    const i = layer.findIndex((e) => e[0] === el);
    if (i >= 0) layer.splice(i, 1);
    __topLayerRemove(el.__id);
  };
  const later = (f) => setTimeout(f, 0);
  // An event's target as an element (a click on a button's text is a click on the button).
  const elOf = (t) => (t && t.nodeType === 3 ? t.parentNode : t);
  const fire = (el, type, init) => el.dispatchEvent(new Event(type, Object.assign({ bubbles: false, cancelable: false }, init || {})));
  const domError = (name, msg) => { try { return new DOMException(msg, name); } catch (e) { const x = new Error(msg); x.name = name; return x; } };
  const focusInside = (el) => {
    const auto = el.querySelector("[autofocus]");
    const first = auto || el.querySelector("button, [href], input:not([type=hidden]), select, textarea, [tabindex]:not([tabindex='-1'])");
    try { (first || el).focus(); } catch (e) {}
  };

  // ToggleEvent (beforetoggle / toggle with oldState and newState).
  if (typeof g.ToggleEvent !== "function") {
    g.ToggleEvent = class ToggleEvent extends Event {
      constructor(type, init) {
        super(type, init);
        this.oldState = (init && init.oldState) || "";
        this.newState = (init && init.newState) || "";
        this.source = (init && init.source) || null;
      }
    };
  }
  // toggle events come after the change, and several changes in a row give one event (from
  // the first old state to the last new state).
  const pendingToggle = new WeakMap();
  const queueToggle = (el, oldState, newState) => {
    const p = pendingToggle.get(el);
    if (p) { p.newState = newState; return; }
    const entry = { oldState, newState };
    pendingToggle.set(el, entry);
    later(() => {
      pendingToggle.delete(el);
      if (entry.oldState !== entry.newState) el.dispatchEvent(new g.ToggleEvent("toggle", { oldState: entry.oldState, newState: entry.newState }));
    });
  };

  // ---------------------------------------------------------------- <dialog>
  const returnValues = new WeakMap();
  // closedby: how a dialog may be closed besides close(): "any" (a click outside too),
  // "closerequest" (Esc), "none". Without the attribute, a modal one is closerequest.
  const closedBy = (d) => {
    const v = (d.getAttribute("closedby") || "").toLowerCase();
    if (v === "any" || v === "closerequest" || v === "none") return v;
    return layer.some((e) => e[0] === d && e[1] === "modal") ? "closerequest" : "none";
  };
  Object.defineProperty(P, "closedBy", {
    get() { return isDialog(this) ? closedBy(this) : undefined; },
    set(v) { this.setAttribute("closedby", String(v)); },
    configurable: true,
  });
  // A dialog about to open: beforetoggle may stop it.
  const mayOpen = (d) => d.dispatchEvent(new g.ToggleEvent("beforetoggle", { cancelable: true, oldState: "closed", newState: "open" }));
  const opener = new WeakMap();
  const dialogApi = {
    show() {
      if (!isDialog(this)) return;
      if (this.hasAttribute("open")) {
        if (layer.some((e) => e[0] === this && e[1] === "modal")) throw domError("InvalidStateError", "The dialog is already open as a modal.");
        return;
      }
      if (!mayOpen(this) || this.hasAttribute("open")) return;
      this.setAttribute("open", "");
      queueToggle(this, "closed", "open");
      focusInside(this);
    },
    showModal() {
      if (!isDialog(this)) return;
      if (this.hasAttribute("open")) {
        if (layer.some((e) => e[0] === this && e[1] === "modal")) return;
        throw domError("InvalidStateError", "The dialog is already open.");
      }
      if (!this.isConnected) throw domError("InvalidStateError", "The dialog isn't in the document.");
      if (this.matches(":popover-open")) throw domError("InvalidStateError", "The dialog is an open popover.");
      if (!mayOpen(this) || this.hasAttribute("open") || !this.isConnected) return;
      opener.set(this, document.activeElement);
      this.setAttribute("open", "");
      queueToggle(this, "closed", "open");
      addTop(this, "modal");
      focusInside(this);
    },
    close(value) {
      if (!isDialog(this) || !this.hasAttribute("open")) return;
      if (value !== undefined && value !== null) returnValues.set(this, String(value));
      this.dispatchEvent(new g.ToggleEvent("beforetoggle", { oldState: "open", newState: "closed" }));
      if (!this.hasAttribute("open")) return;
      this.removeAttribute("open");
      queueToggle(this, "open", "closed");
      removeTop(this);
      const back = opener.get(this);
      opener.delete(this);
      if (back && back.focus) try { back.focus(); } catch (e) {}
      later(() => fire(this, "close"));
    },
    requestClose(value) {
      if (!isDialog(this) || !this.hasAttribute("open")) return;
      if (fire(this, "cancel", { cancelable: true }) && this.hasAttribute("open")) this.close(value);
    },
  };
  for (const [name, f] of Object.entries(dialogApi)) {
    Object.defineProperty(P, name, { value: f, writable: true, configurable: true });
    if (g.HTMLDialogElement) Object.defineProperty(g.HTMLDialogElement.prototype, name, { value: f, writable: true, configurable: true });
  }
  Object.defineProperty(P, "returnValue", {
    get() { return isDialog(this) ? (returnValues.get(this) || "") : undefined; },
    set(v) { if (isDialog(this)) returnValues.set(this, String(v)); },
    configurable: true,
  });
  if (!Object.getOwnPropertyDescriptor(P, "open")) {
    Object.defineProperty(P, "open", {
      get() { return (this.localName === "dialog" || this.localName === "details") ? this.hasAttribute("open") : undefined; },
      set(v) { if (this.localName === "dialog" || this.localName === "details") { if (v) this.setAttribute("open", ""); else this.removeAttribute("open"); } },
      configurable: true,
    });
  }

  // ---------------------------------------------------------------- popovers
  const popoverState = (el) => {
    const v = el.getAttribute("popover");
    if (v === null) return null;
    const l = v.toLowerCase();
    return l === "manual" ? "manual" : l === "hint" ? "hint" : "auto";
  };
  const isOpen = (el) => layer.some((e) => e[0] === el && e[1] === "popover");
  const toggleEvent = (el, type, oldState, newState, cancelable) => {
    let ev;
    try { ev = new (g.ToggleEvent || Event)(type, { cancelable, oldState, newState }); } catch (e) { ev = new Event(type, { cancelable }); }
    if (ev.oldState === undefined) { ev.oldState = oldState; ev.newState = newState; }
    return el.dispatchEvent(ev);
  };
  // Opening an auto popover closes the other auto popovers that aren't its ancestors.
  const closeOtherAutos = (keep) => {
    for (let i = layer.length - 1; i >= 0; i--) {
      const [el, kind] = layer[i];
      if (kind !== "popover" || el === keep || popoverState(el) === "manual") continue;
      if (keep && el.contains(keep)) continue;
      hide(el);
    }
  };
  const show = (el, source) => {
    if (popoverState(el) === null) throw domError("NotSupportedError", "Not a popover.");
    if (isOpen(el)) return;
    if (!el.isConnected) throw domError("InvalidStateError", "The popover isn't in the document.");
    if (!toggleEvent(el, "beforetoggle", "closed", "open", true)) return;
    if (popoverState(el) === "auto") closeOtherAutos(el);
    opener.set(el, source || null);
    addTop(el, "popover");
    later(() => toggleEvent(el, "toggle", "closed", "open", false));
  };
  const hide = (el) => {
    if (!isOpen(el)) return;
    toggleEvent(el, "beforetoggle", "open", "closed", false);
    removeTop(el);
    opener.delete(el);
    later(() => toggleEvent(el, "toggle", "open", "closed", false));
  };
  const popoverApi = {
    showPopover(options) { show(this, options && options.source); },
    hidePopover() {
      if (popoverState(this) === null) throw domError("NotSupportedError", "Not a popover.");
      hide(this);
    },
    togglePopover(force) {
      const want = typeof force === "object" && force !== null ? force.force : force;
      const open = isOpen(this);
      if (want === true || (want === undefined && !open)) show(this, typeof force === "object" && force ? force.source : null);
      else if (want === false || open) this.hidePopover();
      return isOpen(this);
    },
  };
  for (const [name, f] of Object.entries(popoverApi)) Object.defineProperty(P, name, { value: f, writable: true, configurable: true });
  Object.defineProperty(P, "popover", {
    get() { return popoverState(this); },
    set(v) { if (v === null) this.removeAttribute("popover"); else this.setAttribute("popover", String(v)); },
    configurable: true,
  });
  const targetOf = (btn) => {
    if (btn.__popoverTarget && btn.__popoverTarget.isConnected) return btn.__popoverTarget;
    const id = btn.getAttribute("popovertarget");
    return id ? (btn.getRootNode && btn.getRootNode().getElementById ? btn.getRootNode().getElementById(id) : document.getElementById(id)) : null;
  };
  Object.defineProperty(P, "popoverTargetElement", {
    get() { return (this.localName === "button" || this.localName === "input") ? targetOf(this) : undefined; },
    set(el) { this.__popoverTarget = el || null; if (el) this.setAttribute("popovertarget", el.id || ""); else this.removeAttribute("popovertarget"); },
    configurable: true,
  });
  Object.defineProperty(P, "popoverTargetAction", {
    get() { const a = (this.getAttribute("popovertargetaction") || "").toLowerCase(); return a === "show" || a === "hide" ? a : "toggle"; },
    set(v) { this.setAttribute("popovertargetaction", String(v)); },
    configurable: true,
  });

  // ---------------------------------------------------------------- what the page can't do itself
  const outsideModal = (target) => {
    const m = topModal();
    if (!m || !target || target === document || target === g) return false;
    if (m.contains(target)) return false;
    // A popover opened above the modal still works.
    for (let i = layer.length - 1; i >= 0 && layer[i][0] !== m; i--) if (layer[i][0].contains(target)) return false;
    return true;
  };
  // Behind a modal dialog the page is inert: clicks, pointer and key events don't reach it.
  for (const type of ["click", "dblclick", "mousedown", "mouseup", "pointerdown", "pointerup", "auxclick", "contextmenu", "focusin"]) {
    g.addEventListener(type, (e) => {
      if (outsideModal(e.target)) {
        e.stopImmediatePropagation();
        e.preventDefault();
        if (type === "focusin") { const m = topModal(); if (m) focusInside(m); }
      }
    }, true);
  }
  // Clicks: popovertarget buttons, and light dismiss of auto popovers.
  g.addEventListener("click", (e) => {
    const t = elOf(e.target);
    const btn = t && t.closest ? t.closest("button[popovertarget], input[popovertarget]") : null;
    if (btn && !e.defaultPrevented && !btn.disabled) {
      const el = targetOf(btn);
      if (el && popoverState(el) !== null) {
        const a = btn.popoverTargetAction;
        if (a === "show") show(el, btn);
        else if (a === "hide") hide(el);
        else if (isOpen(el)) hide(el);
        else show(el, btn);
        return;
      }
    }
  });
  // A press outside every open auto popover (and the buttons that opened them) closes them:
  // light dismiss.
  g.addEventListener("mousedown", (e) => {
    const t = elOf(e.target);
    // A dialog with closedby=any closes on a press outside it.
    const tp = top();
    if (tp && isDialog(tp[0]) && closedBy(tp[0]) === "any" && !tp[0].contains(t)) tp[0].requestClose();
    for (let i = layer.length - 1; i >= 0; i--) {
      const [el, kind] = layer[i];
      if (kind !== "popover" || popoverState(el) === "manual") continue;
      const src = opener.get(el);
      if (el.contains(t) || (src && src.contains && src.contains(t))) break;
      hide(el);
    }
  }, true);
  // Esc: close the top popover, else cancel the top modal dialog.
  g.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || e.defaultPrevented) return;
    const t = top();
    if (!t) return;
    if (t[1] === "popover") { if (popoverState(t[0]) !== "manual") hide(t[0]); return; }
    if (closedBy(t[0]) !== "none") t[0].requestClose();
  });
  // <form method=dialog>: closes its dialog with the submit button's value instead of sending.
  let lastSubmitter = null;
  // (The browser may send a form as the button goes down, before its click event.)
  for (const type of ["pointerdown", "mousedown", "click"]) {
    g.addEventListener(type, (e) => {
      const t = elOf(e.target);
      const b = t && t.closest ? t.closest("button, input[type=submit], input[type=image]") : null;
      lastSubmitter = b;
    }, true);
  }
  g.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      const a = document.activeElement;
      lastSubmitter = a && a.matches && a.matches("button, input[type=submit], input[type=image]") ? a : null;
    }
  }, true);
  g.addEventListener("submit", (e) => {
    const form = e.target;
    const method = ((e.submitter && e.submitter.getAttribute("formmethod")) || (form.getAttribute && form.getAttribute("method")) || "").toLowerCase();
    if (method !== "dialog" || e.defaultPrevented) return;
    e.preventDefault();
    const d = form.closest("dialog");
    if (!d) return;
    const sub = e.submitter || (lastSubmitter && form.contains(lastSubmitter) ? lastSubmitter : null);
    d.close(sub && sub.hasAttribute("value") ? sub.value : undefined);
  });
  // An element taken out of the document leaves the top layer.
  if (typeof MutationObserver === "function") {
    new MutationObserver(() => {
      for (const [el] of layer.slice()) if (!el.isConnected) { removeTop(el); if (isDialog(el)) el.removeAttribute("open"); }
    }).observe(document, { childList: true, subtree: true, [globalThis.__moSignalOnly]: true }); // records unused
  }
})();
