// Events from the user: the browser calls these when the mouse, keyboard, wheel or focus
// does something on the page. Each event gets the right class (MouseEvent, KeyboardEvent…),
// the standard bubbles/cancelable flags and isTrusted = true.
(function () {
  "use strict";
  const g = globalThis;
  const wrap = g.__wrapFn;

  if (!g.InputEvent) {
    g.InputEvent = class InputEvent extends UIEvent {
      constructor(type, init = {}) {
        super(type, init);
        this.data = init.data ?? null;
        this.inputType = init.inputType ?? "";
        this.isComposing = !!init.isComposing;
        this.dataTransfer = init.dataTransfer ?? null;
      }
      getTargetRanges() { return []; }
    };
  }
  if (!g.SubmitEvent) g.SubmitEvent = class SubmitEvent extends Event { constructor(type, init = {}) { super(type, init); this.submitter = init.submitter ?? null; } };
  if (!g.CompositionEvent) g.CompositionEvent = class CompositionEvent extends UIEvent { constructor(type, init = {}) { super(type, init); this.data = init.data ?? ""; } };

  // AnimationEvent and TransitionEvent: read-only fields, a finite elapsedTime.
  function timedEvent(name, field) {
    const fields = new WeakMap();
    const C = class extends Event {
      constructor(type, init) {
        if (arguments.length < 1) throw new TypeError(`Failed to construct '${name}': 1 argument required, but only 0 present.`);
        init = init ?? {};
        const t = init.elapsedTime === undefined ? 0 : Number(init.elapsedTime);
        if (!Number.isFinite(t)) throw new TypeError(`Failed to construct '${name}': elapsedTime is not a finite number.`);
        super(type, init);
        fields.set(this, {
          name: init[field] === undefined ? "" : String(init[field]),
          elapsedTime: t,
          pseudoElement: init.pseudoElement === undefined ? "" : String(init.pseudoElement),
          animation: init.animation ?? null,
        });
      }
      get [field]() { return fields.get(this)?.name ?? ""; }
      get elapsedTime() { return fields.get(this)?.elapsedTime ?? 0; }
      get pseudoElement() { return fields.get(this)?.pseudoElement ?? ""; }
    };
    if (field === "animationName") Object.defineProperty(C.prototype, "animation", { configurable: true, enumerable: true, get() { return fields.get(this)?.animation ?? null; } });
    Object.defineProperty(C, "name", { value: name });
    return C;
  }
  if (!g.AnimationEvent) g.AnimationEvent = timedEvent("AnimationEvent", "animationName");
  if (!g.TransitionEvent) g.TransitionEvent = timedEvent("TransitionEvent", "propertyName");

  // type -> [class, bubbles, cancelable]
  const M = MouseEvent, P = PointerEvent, K = KeyboardEvent, F = FocusEvent;
  const KINDS = {
    click: [P, true, true], auxclick: [P, true, true], contextmenu: [P, true, true], dblclick: [M, true, true],
    mousedown: [M, true, true], mouseup: [M, true, true], mousemove: [M, true, true],
    mouseover: [M, true, true], mouseout: [M, true, true], mouseenter: [M, false, false], mouseleave: [M, false, false],
    pointerdown: [P, true, true], pointerup: [P, true, true], pointermove: [P, true, true],
    pointerover: [P, true, true], pointerout: [P, true, true], pointerenter: [P, false, false], pointerleave: [P, false, false],
    wheel: [WheelEvent, true, true],
    keydown: [K, true, true], keyup: [K, true, true], keypress: [K, true, true],
    focus: [F, false, false], blur: [F, false, false], focusin: [F, true, false], focusout: [F, true, false],
    input: [g.InputEvent, true, false], beforeinput: [g.InputEvent, true, true],
    change: [Event, true, false], submit: [g.SubmitEvent, true, true], reset: [Event, true, true],
    select: [Event, true, false], invalid: [Event, false, true], toggle: [Event, false, false],
    scroll: [Event, false, false], scrollend: [Event, false, false],
    animationstart: [g.AnimationEvent, true, false], animationend: [g.AnimationEvent, true, false], animationiteration: [g.AnimationEvent, true, false], animationcancel: [g.AnimationEvent, true, false],
    transitionrun: [g.TransitionEvent, true, false], transitionstart: [g.TransitionEvent, true, false], transitionend: [g.TransitionEvent, true, false], transitioncancel: [g.TransitionEvent, true, false],
    compositionstart: [g.CompositionEvent, true, true], compositionupdate: [g.CompositionEvent, true, false], compositionend: [g.CompositionEvent, true, false],
  };

  // The focused element, as the page sees it (document.activeElement).
  let active = null;
  const connected = (el) => el && el.isConnected;
  const docProto = Object.getPrototypeOf(document);
  Object.defineProperty(docProto, "activeElement", {
    configurable: true,
    get() { return connected(active) ? active : this.body || this.documentElement; },
  });
  const hasFocus = docProto.hasFocus;
  docProto.hasFocus = function () { return hasFocus ? hasFocus.call(this) : true; };

  // Where the mouse is (client coordinates), for events that don't carry their own.
  let mouse = { clientX: 0, clientY: 0, screenX: 0, screenY: 0 };
  let buttons = 0;

  function make(type, init) {
    const k = KINDS[type] || [Event, true, true];
    const opts = Object.assign({ bubbles: k[1], cancelable: k[2], composed: k[0] !== Event && k[0] !== g.AnimationEvent && k[0] !== g.TransitionEvent, view: g }, init);
    if (k[0] === M || k[0] === P || k[0] === WheelEvent) {
      for (const key of ["clientX", "clientY", "screenX", "screenY"]) if (opts[key] === undefined) opts[key] = mouse[key];
      if (opts.buttons === undefined) opts.buttons = buttons;
      if (k[0] === P) { opts.pointerId = 1; opts.pointerType = "mouse"; opts.isPrimary = true; }
      if (type === "click" || type === "dblclick" || type === "auxclick") opts.detail = opts.detail ?? (type === "dblclick" ? 2 : 1);
    }
    const ev = new k[0](type, opts);
    Object.defineProperty(ev, "isTrusted", { value: true, enumerable: true });
    return ev;
  }

  function fire(target, type, init) {
    if (!target) return true;
    return target.dispatchEvent(make(type, init));
  }

  // The browser's entry point for one event at a node (-1: the focused element).
  // Returns false if the page cancelled it.
  function userEvent(id, type, init) {
    let target = id < 0 ? document.activeElement || document : wrap(id);
    if (!target) return true;
    init = init || {};
    if (type === "focus") {
      if (active === target) return true;
      const old = connected(active) ? active : null;
      if (old) { active = null; fire(old, "blur", { relatedTarget: target }); fire(old, "focusout", { relatedTarget: target }); }
      active = target;
      fire(target, "focus", { relatedTarget: old });
      fire(target, "focusin", { relatedTarget: old });
      return true;
    }
    if (type === "blur") {
      if (active !== target) return true;
      active = null;
      fire(target, "blur", init);
      fire(target, "focusout", init);
      return true;
    }
    if (type === "scroll" && (id === 0 || target === document)) {
      // Page scrolling: the document's scroll event bubbles to the window.
      return document.dispatchEvent(Object.defineProperty(new Event("scroll", { bubbles: true }), "isTrusted", { value: true }));
    }
    if (type === "mousedown") buttons |= 1 << (init.button === 2 ? 1 : init.button === 1 ? 2 : 0);
    if (type === "mouseup") buttons &= ~(1 << (init.button === 2 ? 1 : init.button === 1 ? 2 : 0));
    if (type === "mousedown" || type === "mouseup") {
      // Pointer events come first; cancelling pointerdown doesn't stop the mouse event.
      fire(target, type === "mousedown" ? "pointerdown" : "pointerup", init);
    }
    if (type === "input" && init.inputType === undefined) init.inputType = "insertText";
    return fire(target, type, init);
  }
  g.__userEvent = userEvent;
  g.__dispatch = (id, type) => userEvent(id, type, {});

  // The element under the mouse changed and/or the mouse moved.
  let over = null;
  const chain = (el) => { const out = []; for (let n = el; n && n.nodeType === 1; n = n.parentNode) out.push(n); return out; };
  g.__pointerMove = function (id, x, y, mods) {
    mouse = { clientX: x, clientY: y, screenX: x, screenY: y };
    const init = Object.assign({}, mods || {});
    const now = id >= 0 ? wrap(id) : null;
    if (now !== over) {
      const old = over && over.isConnected ? over : null;
      over = now;
      const oldChain = chain(old), newChain = chain(now);
      if (old) {
        fire(old, "pointerout", Object.assign({ relatedTarget: now }, init));
        fire(old, "mouseout", Object.assign({ relatedTarget: now }, init));
        for (const n of oldChain) if (!newChain.includes(n)) { fire(n, "pointerleave", Object.assign({ relatedTarget: now }, init)); fire(n, "mouseleave", Object.assign({ relatedTarget: now }, init)); }
      }
      if (now) {
        fire(now, "pointerover", Object.assign({ relatedTarget: old }, init));
        fire(now, "mouseover", Object.assign({ relatedTarget: old }, init));
        for (const n of newChain.slice().reverse()) if (!oldChain.includes(n)) { fire(n, "pointerenter", Object.assign({ relatedTarget: old }, init)); fire(n, "mouseenter", Object.assign({ relatedTarget: old }, init)); }
      }
    }
    if (now) {
      fire(now, "pointermove", init);
      fire(now, "mousemove", init);
    }
  };

  // A promise was rejected and nothing handled it by the end of the task. Returns true if
  // the error should still be reported (the page didn't call preventDefault()).
  if (!g.PromiseRejectionEvent) {
    g.PromiseRejectionEvent = class PromiseRejectionEvent extends Event {
      constructor(type, init = {}) { super(type, init); this.promise = init.promise; this.reason = init.reason; }
    };
  }
  g.__unhandledRejection = function (promise, reason) {
    const ev = new g.PromiseRejectionEvent("unhandledrejection", { cancelable: true, promise, reason });
    Object.defineProperty(ev, "isTrusted", { value: true });
    EventTarget.prototype.__fire.call(g, ev);
    return !ev.defaultPrevented;
  };

  // After each layout: scroll and resize events, then observers (IntersectionObserver…).
  let lastScroll = null, lastSize = null;
  const hooks = (g.__layoutHooks ||= []);
  g.__afterLayout = function () {
    const y = g.scrollY, size = g.innerWidth + "x" + g.innerHeight;
    if (lastScroll !== null && y !== lastScroll) userEvent(0, "scroll", {});
    if (lastSize !== null && size !== lastSize) EventTarget.prototype.__fire.call(g, Object.defineProperty(new Event("resize"), "isTrusted", { value: true }));
    lastScroll = y;
    lastSize = size;
    for (const h of hooks) { try { h(); } catch (e) { console.error(String(e)); } }
  };
})();
