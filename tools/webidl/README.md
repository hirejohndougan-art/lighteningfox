# WebIDL bindings generator

Generates Ironswift's native DOM bindings (`src/bindings/generated.rs`) from the WebIDL in
the web specs.

```
python3 tools/webidl/gen.py            # regenerate src/bindings/generated.rs
python3 tools/webidl/gen.py --check    # fail if it is out of date (cargo test runs this)
python3 tools/webidl/gen.py --report   # per interface: members native / members in the IDL
```

## Files

- `idl/` — the spec IDL, copied unchanged from [w3c/webref](https://github.com/w3c/webref)
  (`ed/idl/`, commit `47c79cfd5aaf6ed4b85b22226586d41d64343f16`, 2026-11-17). To update, copy
  the newer files over these and regenerate. Add another spec's file here and to the `idl`
  list in `bindings.json` to bind its interfaces.
- `webidl.py` — the WebIDL parser: interfaces, partial interfaces, mixins and `includes`,
  dictionaries, enums, typedefs, callbacks, namespaces, extended attributes, union and
  generic types. It parses 356 of webref's 358 IDL files (the other two aren't valid WebIDL).
- `bindings.json` — which members are native and the Rust function that implements each
  (`"getAttribute": "get_attribute"`, or `{"fn": "substring_data", "throws": true}` for one
  that returns `Result<T, conv::DomError>` and so can throw a DOMException). A member may be
  listed on an interface that inherits it from the IDL (Element binds Node's `nodeName`).
- `gen.py` — the generator.

## What the generated glue does

For each member, as the WebIDL spec says: checks `this` is a node (TypeError otherwise);
throws "N arguments required" for too few arguments; converts each argument by its IDL type
(DOMString/USVString, nullable strings, `[LegacyNullToEmptyString]`, boolean, all integer types
with wrap-around, `[EnforceRange]` and `[Clamp]`, restricted and unrestricted doubles, enums,
nodes, optional arguments with and without defaults); converts the result back (strings,
numbers, booleans, nodes, node lists, string lists); and turns errors into DOMExceptions. The
member table gives each function the IDL's `length` and getters the name `get <name>`.

The implementations in `src/bindings/dom.rs` are plain typed Rust (`fn get_attribute(this:
Option<NodeId>, name: String) -> Option<String>`); each generated function's doc comment shows
the IDL line and the function it calls.

## Adding a native member

1. Add it to `bindings.json`.
2. Write the function in `src/bindings/dom.rs`. Argument and result types follow from the IDL:
   strings → `String` / `Option<String>`, integers → `u32`, `i32`…, nodes → `Option<NodeId>`,
   `NodeList`/`HTMLCollection` → `Vec<NodeId>`.
3. Run `python3 tools/webidl/gen.py` and build. A type the generator doesn't handle yet
   (unions, dictionaries, callbacks, variadic arguments, overloads, non-node interfaces) stops
   it with a message saying so.

Members that change the tree or attributes stay in JavaScript (`prelude.js`) on purpose:
custom elements and mutation observers hook the JavaScript mutation path.
