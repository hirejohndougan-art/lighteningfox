// WebAssembly loading (task P1c): compileStreaming and instantiateStreaming, a
// WebAssembly.Module sent with postMessage (src/clone.js calls __wasmShare / __wasmAdopt,
// src/wasm/share.rs), and the glue for `.wasm` ES modules (src/wasm/esm.rs). Runs after wasm.js.
(() => {
  const g = globalThis;
  const WA = g.WebAssembly;
  if (!WA) return;
  const moduleId = g.__wasmModuleId, moduleFromId = g.__wasmModuleFromId;
  const shareNative = __wasmShareNative, adoptNative = __wasmAdoptNative;
  const compile = WA.compile, instantiate = WA.instantiate, Instance = WA.Instance;

  // A Response's bytes, checked the way the spec asks: a Response (or a promise of one),
  // served as application/wasm, readable (not an opaque cross-origin response), ok and unread.
  // Bodies arrive whole here, so compiling starts once the last byte is in.
  function responseBytes(what, source) {
    return Promise.resolve(source).then((r) => {
      if (typeof g.Response !== "function" || !(r instanceof g.Response))
        throw new TypeError(`WebAssembly.${what}(): Argument 0 must be provided and must be a Response`);
      // Exactly application/wasm (any case): no parameters, not even an empty one.
      const type = (r.headers.get("content-type") || "").trim().toLowerCase();
      if (type !== "application/wasm")
        throw new TypeError(`WebAssembly.${what}(): Incorrect response MIME type. Expected 'application/wasm'.`);
      if (r.type !== "basic" && r.type !== "cors" && r.type !== "default")
        throw new TypeError(`WebAssembly.${what}(): Response.type must be 'basic', 'cors' or 'default'.`);
      if (!r.ok) throw new TypeError(`WebAssembly.${what}(): HTTP status code is not ok`);
      if (r.bodyUsed) throw new TypeError(`WebAssembly.${what}(): Response body has already been used`);
      return r.arrayBuffer();
    });
  }

  const added = {
    compileStreaming(source) {
      return responseBytes("compileStreaming", source).then((bytes) => compile.call(WA, bytes));
    },
    instantiateStreaming(source, ...rest) {
      const importObject = rest[0];
      return responseBytes("instantiateStreaming", source).then((bytes) => instantiate.call(WA, bytes, importObject));
    },
  };
  // Namespace operations are enumerable (WebIDL).
  for (const [name, value] of Object.entries(added))
    Object.defineProperty(WA, name, { value, writable: true, configurable: true, enumerable: true });

  const hidden = {
    // postMessage: a token for the module (its bytes wait in share.rs), and a module from one.
    __wasmShare: (m) => {
      const id = moduleId(m);
      if (id === undefined) throw new TypeError("A WebAssembly.Module could not be cloned.");
      return shareNative(id);
    },
    __wasmAdopt: (token) => moduleFromId(adoptNative(token)),
    // A .wasm ES module's instance exports (esm.rs writes the call).
    __wasmEsmInstance: (id, imports) => new Instance(moduleFromId(id), imports).exports,
  };
  for (const [name, value] of Object.entries(hidden))
    Object.defineProperty(g, name, { value, writable: true, configurable: true, enumerable: false });
})();
