// The WebAssembly JavaScript API (task P1a): WebAssembly.Module, Instance, validate,
// compile, instantiate and the CompileError / LinkError / RuntimeError classes, on the
// natives in src/wasm/mod.rs (wasmi underneath).
// Still to come: Memory, Table, Global (P1b); compileStreaming / instantiateStreaming,
// .wasm ES modules and CSP wasm-unsafe-eval (P1c).
(() => {
  const compileNative = __wasmCompile, validateNative = __wasmValidate, importsNative = __wasmImports,
    exportsNative = __wasmExports, customNative = __wasmCustomSections, instantiateNative = __wasmInstantiate;

  function errorClass(name) {
    const C = class extends Error {
      constructor(message, options) {
        super(message, options);
      }
    };
    Object.defineProperty(C, "name", { value: name, configurable: true });
    Object.defineProperty(C, "length", { value: 1, configurable: true });
    Object.defineProperty(C.prototype, "name", { value: name, writable: true, configurable: true });
    return C;
  }
  const CompileError = errorClass("CompileError");
  const LinkError = errorClass("LinkError");
  const RuntimeError = errorClass("RuntimeError");
  __wasmErrors(CompileError, LinkError, RuntimeError);

  // A BufferSource's bytes as a Uint8Array (copied out of shared memory).
  function bytesOf(source) {
    if (source instanceof ArrayBuffer) return new Uint8Array(source);
    if (typeof SharedArrayBuffer === "function" && source instanceof SharedArrayBuffer) return new Uint8Array(source).slice();
    if (ArrayBuffer.isView(source)) {
      const view = new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
      return source.buffer instanceof ArrayBuffer ? view : view.slice();
    }
    throw new TypeError("WebAssembly: first argument must be an ArrayBuffer or typed array");
  }

  const moduleIds = new WeakMap();
  const instanceExports = new WeakMap();

  function idOf(module, what) {
    const id = typeof module === "object" && module !== null ? moduleIds.get(module) : undefined;
    if (id === undefined) throw new TypeError(what + ": argument must be a WebAssembly.Module");
    return id;
  }

  class Module {
    constructor(bytes) {
      moduleIds.set(this, compileNative(bytesOf(bytes)));
    }
    static exports(module) {
      return exportsNative(idOf(module, "WebAssembly.Module.exports")).map(([name, kind]) => ({ name, kind }));
    }
    static imports(module) {
      return importsNative(idOf(module, "WebAssembly.Module.imports")).map(([module, name, kind]) => ({ module, name, kind }));
    }
    static customSections(module, sectionName) {
      const id = idOf(module, "WebAssembly.Module.customSections");
      if (sectionName === undefined) throw new TypeError("WebAssembly.Module.customSections: a section name is required");
      return customNative(id, String(sectionName));
    }
  }
  Object.defineProperty(Module.prototype, Symbol.toStringTag, { value: "WebAssembly.Module", configurable: true });

  // Read the import object the way the spec does (in the module's import order), then
  // instantiate; the exports object is frozen with a null prototype.
  function instantiateModule(id, importObject) {
    const wanted = importsNative(id);
    if (importObject !== undefined && (importObject === null || (typeof importObject !== "object" && typeof importObject !== "function")))
      throw new TypeError("WebAssembly.Instance: imports argument must be an object");
    if (wanted.length > 0 && importObject === undefined)
      throw new TypeError("WebAssembly.Instance: imports argument must be present and must be an object");
    const values = [];
    for (const [moduleName, name, kind] of wanted) {
      const ns = importObject[moduleName];
      if (ns === null || (typeof ns !== "object" && typeof ns !== "function"))
        throw new TypeError(`WebAssembly.Instance: import module "${moduleName}" must be an object`);
      const v = ns[name];
      if (kind === "function" && typeof v !== "function")
        throw new LinkError(`WebAssembly.Instance: import "${moduleName}" "${name}": function import requires a callable`);
      values.push(v);
    }
    const list = instantiateNative(id, values);
    const exports = Object.create(null);
    for (let i = 0; i < list.length; i += 2) exports[list[i]] = list[i + 1];
    return Object.freeze(exports);
  }

  class Instance {
    constructor(module, importObject) {
      instanceExports.set(this, instantiateModule(idOf(module, "WebAssembly.Instance"), importObject));
    }
    get exports() {
      const e = instanceExports.get(this);
      if (e === undefined) throw new TypeError("WebAssembly.Instance.prototype.exports: not an Instance");
      return e;
    }
  }
  Object.defineProperty(Instance.prototype, Symbol.toStringTag, { value: "WebAssembly.Instance", configurable: true });
  Object.defineProperty(Instance.prototype, "exports", { enumerable: true });

  // Work for a promise: done in a later job, like a browser compiling off the main thread.
  function later(work) {
    return new Promise((resolve, reject) => {
      Promise.resolve().then(() => {
        try {
          resolve(work());
        } catch (e) {
          reject(e);
        }
      });
    });
  }

  const WebAssembly = {
    validate(bytes) {
      return validateNative(bytesOf(bytes));
    },
    compile(bytes) {
      let copy;
      try {
        copy = bytesOf(bytes).slice();
      } catch (e) {
        return Promise.reject(e);
      }
      return later(() => new Module(copy));
    },
    instantiate(source, importObject) {
      if (typeof source === "object" && source !== null && moduleIds.has(source))
        return later(() => new Instance(source, importObject));
      let copy;
      try {
        copy = bytesOf(source).slice();
      } catch (e) {
        return Promise.reject(e);
      }
      return later(() => new Module(copy)).then((module) => later(() => ({ module, instance: new Instance(module, importObject) })));
    },
    Module,
    Instance,
    CompileError,
    LinkError,
    RuntimeError,
  };
  for (const key of Object.keys(WebAssembly)) {
    Object.defineProperty(WebAssembly, key, { enumerable: false });
  }
  Object.defineProperty(WebAssembly, Symbol.toStringTag, { value: "WebAssembly", configurable: true });
  Object.defineProperty(globalThis, "WebAssembly", { value: WebAssembly, writable: true, configurable: true, enumerable: false });
})();
