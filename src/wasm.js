// The WebAssembly JavaScript API (task P1a): WebAssembly.Module, Instance, validate,
// compile, instantiate and the CompileError / LinkError / RuntimeError classes, on the
// natives in src/wasm/mod.rs (wasmi underneath).
// Memory, Table and Global (task P1b) work on slot numbers (src/wasm/objects.rs).
// compileStreaming / instantiateStreaming and the glue for postMessage and .wasm ES modules
// are in wasm_loading.js (P1c).
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

  // [[EnforceRange]] unsigned long (and the like): a whole number in [0, max].
  function enforceRange(v, what, max = 0xffffffff) {
    const n = Number(v);
    if (!Number.isFinite(n)) throw new TypeError(what + " must be a finite number");
    const t = Math.trunc(n);
    if (t < 0 || t > max) throw new TypeError(what + " is out of range");
    return t;
  }

  // Memory, Table and Global objects: one per slot (per kind), made here or for an export.
  const slotOf = new WeakMap();
  const objectFor = { memory: new Map(), table: new Map(), global: new Map() };
  function wrap(kind, proto, slot) {
    let o = objectFor[kind].get(slot);
    if (!o) {
      o = Object.create(proto);
      slotOf.set(o, { kind, slot });
      objectFor[kind].set(slot, o);
    }
    return o;
  }
  function slotFor(o, kind, what) {
    const s = typeof o === "object" && o !== null ? slotOf.get(o) : undefined;
    if (!s || s.kind !== kind) throw new TypeError(what + ": receiver is not a WebAssembly." + kind[0].toUpperCase() + kind.slice(1));
    return s.slot;
  }
  function descriptorOf(d, what) {
    if (d === null || (typeof d !== "object" && typeof d !== "function")) throw new TypeError(what + ": descriptor must be an object");
    return d;
  }

  class Memory {
    constructor(descriptor) {
      const d = descriptorOf(descriptor, "WebAssembly.Memory");
      const initialRaw = d.initial !== undefined ? d.initial : d.minimum;
      if (initialRaw === undefined) throw new TypeError("WebAssembly.Memory: descriptor needs 'initial'");
      const initial = enforceRange(initialRaw, "WebAssembly.Memory initial");
      const maximum = d.maximum === undefined ? -1 : enforceRange(d.maximum, "WebAssembly.Memory maximum");
      if (d.shared) {
        if (maximum < 0) throw new TypeError("WebAssembly.Memory: shared memory needs a maximum");
        throw new TypeError("WebAssembly.Memory: shared memory is not supported");
      }
      const slot = __wasmMemoryNew(initial, maximum);
      slotOf.set(this, { kind: "memory", slot });
      objectFor.memory.set(slot, this);
    }
    get buffer() {
      return __wasmMemoryBuffer(slotFor(this, "memory", "WebAssembly.Memory.buffer"));
    }
    grow(delta) {
      const slot = slotFor(this, "memory", "WebAssembly.Memory.grow");
      return __wasmMemoryGrow(slot, enforceRange(delta, "WebAssembly.Memory.grow() delta"));
    }
  }

  class Table {
    constructor(descriptor, value) {
      const d = descriptorOf(descriptor, "WebAssembly.Table");
      const element = String(d.element);
      if (element !== "anyfunc" && element !== "funcref" && element !== "externref")
        throw new TypeError("WebAssembly.Table: descriptor 'element' must be \"anyfunc\" or \"externref\"");
      const initialRaw = d.initial !== undefined ? d.initial : d.minimum;
      if (initialRaw === undefined) throw new TypeError("WebAssembly.Table: descriptor needs 'initial'");
      const initial = enforceRange(initialRaw, "WebAssembly.Table initial");
      const maximum = d.maximum === undefined ? -1 : enforceRange(d.maximum, "WebAssembly.Table maximum");
      const slot = __wasmTableNew(element, initial, maximum, value);
      slotOf.set(this, { kind: "table", slot });
      objectFor.table.set(slot, this);
    }
    get length() {
      return __wasmTableLength(slotFor(this, "table", "WebAssembly.Table.length"));
    }
    get(index) {
      const slot = slotFor(this, "table", "WebAssembly.Table.get");
      return __wasmTableGet(slot, enforceRange(index, "WebAssembly.Table.get() index"));
    }
    set(index, ...value) {
      const slot = slotFor(this, "table", "WebAssembly.Table.set");
      const i = enforceRange(index, "WebAssembly.Table.set() index");
      if (value.length) __wasmTableSet(slot, i, value[0]);
      else __wasmTableSet(slot, i);
    }
    grow(delta, ...value) {
      const slot = slotFor(this, "table", "WebAssembly.Table.grow");
      const n = enforceRange(delta, "WebAssembly.Table.grow() delta");
      return value.length ? __wasmTableGrow(slot, n, value[0]) : __wasmTableGrow(slot, n);
    }
  }

  const VALUE_TYPES = ["i32", "i64", "f32", "f64", "v128", "externref", "anyfunc", "funcref"];
  class Global {
    constructor(descriptor, ...v) {
      const d = descriptorOf(descriptor, "WebAssembly.Global");
      const mutable = !!d.mutable;
      const type = String(d.value);
      if (!VALUE_TYPES.includes(type)) throw new TypeError("WebAssembly.Global: descriptor 'value' must be a WebAssembly value type");
      if (type === "v128") throw new TypeError("WebAssembly.Global: v128 globals can't be made from JavaScript");
      let value = v.length ? v[0] : undefined;
      // A missing value is the type's default.
      if (value === undefined) value = type === "i64" ? 0n : type === "anyfunc" || type === "funcref" ? null : type === "externref" ? undefined : 0;
      const slot = __wasmGlobalNew(type, mutable, value);
      slotOf.set(this, { kind: "global", slot });
      objectFor.global.set(slot, this);
    }
    get value() {
      return __wasmGlobalGet(slotFor(this, "global", "WebAssembly.Global.value"));
    }
    set value(v) {
      const slot = slotFor(this, "global", "WebAssembly.Global.value");
      __wasmGlobalSet(slot, v);
    }
    valueOf() {
      return __wasmGlobalGet(slotFor(this, "global", "WebAssembly.Global.valueOf"));
    }
  }
  for (const [C, tag] of [[Memory, "WebAssembly.Memory"], [Table, "WebAssembly.Table"], [Global, "WebAssembly.Global"]]) {
    Object.defineProperty(C.prototype, Symbol.toStringTag, { value: tag, configurable: true });
    for (const key of Object.getOwnPropertyNames(C.prototype)) {
      if (key !== "constructor") Object.defineProperty(C.prototype, key, { enumerable: true });
    }
  }
  const PROTOS = { memory: Memory.prototype, table: Table.prototype, global: Global.prototype };

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
    const values = [], slots = [];
    for (const [moduleName, name, kind] of wanted) {
      const ns = importObject[moduleName];
      if (ns === null || (typeof ns !== "object" && typeof ns !== "function"))
        throw new TypeError(`WebAssembly.Instance: import module "${moduleName}" must be an object`);
      const v = ns[name];
      const where = `WebAssembly.Instance: import "${moduleName}" "${name}"`;
      if (kind === "function" && typeof v !== "function") throw new LinkError(where + ": function import requires a callable");
      const s = typeof v === "object" && v !== null ? slotOf.get(v) : undefined;
      if ((kind === "memory" || kind === "table") && (!s || s.kind !== kind))
        throw new LinkError(where + ": " + kind + " import must be a WebAssembly." + kind[0].toUpperCase() + kind.slice(1));
      values.push(v);
      slots.push(s && s.kind === kind ? s.slot : -1);
    }
    const list = instantiateNative(id, values, slots);
    const exports = Object.create(null);
    for (let i = 0; i < list.length; i += 3) {
      const kind = list[i + 1];
      exports[list[i]] = kind === "function" ? list[i + 2] : wrap(kind, PROTOS[kind], list[i + 2]);
    }
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
    Memory,
    Table,
    Global,
    CompileError,
    LinkError,
    RuntimeError,
  };
  for (const key of Object.keys(WebAssembly)) {
    Object.defineProperty(WebAssembly, key, { enumerable: false });
  }
  Object.defineProperty(WebAssembly, Symbol.toStringTag, { value: "WebAssembly", configurable: true });
  Object.defineProperty(globalThis, "WebAssembly", { value: WebAssembly, writable: true, configurable: true, enumerable: false });

  // For wasm_loading.js (P1c: postMessage and .wasm ES modules): module objects to ids and back.
  const hooks = {
    __wasmModuleId: (m) => (typeof m === "object" && m !== null ? moduleIds.get(m) : undefined),
    __wasmModuleFromId: (id) => {
      const m = Object.create(Module.prototype);
      moduleIds.set(m, id);
      return m;
    },
  };
  for (const [name, value] of Object.entries(hooks))
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true, enumerable: false });
})();
