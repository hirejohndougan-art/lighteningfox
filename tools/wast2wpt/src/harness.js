// Harness for the pages tools/wast2wpt makes from WebAssembly spec tests: every step is one
// testharness.js test, run through the JavaScript API (WebAssembly.Module, Instance…).
"use strict";
(() => {
  const g = globalThis;
  const bin = (s) => Uint8Array.from(s, (c) => c.charCodeAt(0));
  // spectest is a wasm module (SPECTEST_WASM, written by wast2wpt), so its imports have exact types.
  const spectest = new WebAssembly.Instance(new WebAssembly.Module(bin(SPECTEST_WASM))).exports;
  // The import object: registered instances by name.
  const registry = { spectest };
  const instances = {}, modules = {};
  let current = null, lastModule = null;
  const refs = new Map();

  const show = (v) => (typeof v === "bigint" ? v + "n" : Object.is(v, -0) ? "-0" : typeof v === "function" ? "function" : String(v));

  Object.assign(g, {
    externref(n) {
      if (!refs.has(n)) refs.set(n, { externref: n });
      return refs.get(n);
    },
    step(name, fn) {
      test(fn, name);
    },
    instantiate(bytes) {
      return new WebAssembly.Instance(new WebAssembly.Module(bin(bytes)), registry).exports;
    },
    // A module directive: later steps use it (and none if it failed).
    define(id, make) {
      current = null;
      current = make();
      if (id !== null) instances[id] = current;
    },
    defineModule(id, bytes) {
      lastModule = new WebAssembly.Module(bin(bytes));
      if (id !== null) modules[id] = lastModule;
    },
    instantiateDefined(moduleId, id) {
      const m = moduleId !== null ? modules[moduleId] : lastModule;
      return () => new WebAssembly.Instance(m, registry).exports;
    },
    register(name, id) {
      const e = id !== null ? instances[id] : current;
      assert_true(!!e, "a module to register");
      registry[name] = e;
    },
    invoke(id, name, args) {
      const e = id !== null ? instances[id] : current;
      assert_true(!!e, "the module was instantiated");
      assert_equals(typeof e[name], "function", `export ${name}`);
      return e[name](...args);
    },
    getGlobal(id, name) {
      const e = id !== null ? instances[id] : current;
      assert_true(!!e, "the module was instantiated");
      return e[name] instanceof WebAssembly.Global ? e[name].value : e[name];
    },
    results(v, n) {
      return n === 0 ? [] : n === 1 ? [v] : Array.from(v);
    },
    check(r, ok) {
      assert_true(ok, "got " + r.map(show).join(", "));
    },
    throwsRuntimeError(fn, message) {
      let thrown;
      try {
        fn();
      } catch (e) {
        thrown = e;
      }
      assert_true(thrown instanceof WebAssembly.RuntimeError, `expected a RuntimeError (${message}), got ${thrown}`);
    },
    throwsExhaustion(fn, message) {
      let thrown;
      try {
        fn();
      } catch (e) {
        thrown = e;
      }
      assert_true(thrown instanceof RangeError, `expected a RangeError (${message}), got ${thrown}`);
    },
    invalid(bytes) {
      const b = bin(bytes);
      assert_false(WebAssembly.validate(b), "validate");
      assert_throws_js(WebAssembly.CompileError, () => new WebAssembly.Module(b));
    },
    unlinkable(bytes) {
      const m = new WebAssembly.Module(bin(bytes));
      let thrown;
      try {
        new WebAssembly.Instance(m, registry);
      } catch (e) {
        thrown = e;
      }
      // A missing import module is a TypeError in the JS API, a wrong import a LinkError.
      assert_true(thrown instanceof WebAssembly.LinkError || thrown instanceof TypeError, `expected a LinkError, got ${thrown}`);
    },
  });
})();
