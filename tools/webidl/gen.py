#!/usr/bin/env python3
"""Generate Ironswift's native DOM bindings from the WebIDL in the specs.

    python3 tools/webidl/gen.py            # write src/bindings/generated.rs
    python3 tools/webidl/gen.py --check    # fail if generated.rs is out of date (tests run this)
    python3 tools/webidl/gen.py --report   # per interface: members bound natively / in the IDL

Reads the spec IDL in tools/webidl/idl/ (copied from w3c/webref, see README.md) and
tools/webidl/bindings.json, which names the members that have a Rust implementation and where
it is. For each one it writes the glue the WebIDL spec asks for: the `this` check, the
"N arguments required" TypeError, each argument converted by its IDL type (strings, nullable
values, optional arguments and their defaults, the integer types with [EnforceRange]/[Clamp],
restricted doubles, enums, nodes), the result converted back, and the member table with each
function's `length` and name. The implementations are plain typed Rust functions
(src/bindings/dom.rs) that never see a JavaScript value.
"""
import glob, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
OUT = os.path.join(ROOT, "src", "bindings", "generated.rs")
sys.dont_write_bytecode = True
sys.path.insert(0, HERE)
import webidl  # noqa: E402

STRINGS = {"DOMString", "USVString", "ByteString", "CSSOMString"}
INTS = {  # IDL type: (bits, signed, rust type)
    "byte": (8, True, "i8"), "octet": (8, False, "u8"),
    "short": (16, True, "i16"), "unsigned short": (16, False, "u16"),
    "long": (32, True, "i32"), "unsigned long": (32, False, "u32"),
    "long long": (64, True, "i64"), "unsigned long long": (64, False, "u64"),
}
DOUBLES = {"double": False, "float": False, "unrestricted double": True, "unrestricted float": True}
NODE_LISTS = {"NodeList", "HTMLCollection"}


class GenError(Exception):
    pass


def snake(name):
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name)
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", s)
    return s.lower()


KEYWORDS = {"type", "ref", "self", "match", "in", "fn", "mod", "use", "move", "loop", "where", "impl", "box", "async", "await", "dyn", "final", "override", "priv", "static", "struct", "trait", "true", "false", "let", "const", "crate", "super", "enum", "extern", "for", "if", "else", "while", "return", "pub", "mut", "unsafe", "abstract", "become", "do", "macro", "typeof", "unsized", "virtual", "yield", "try", "gen"}


def var(name):
    s = snake(name)
    return s + "_" if s in KEYWORDS else s


def rust_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


class Gen:
    def __init__(self, model, config):
        self.m = model
        self.cfg = config
        self.fns = []

    def is_node(self, t):
        return t.name in self.m.interfaces and self.m.inherits_from(t.name, "Node")

    def find(self, iface, name):
        """The IDL member `name` of `iface` or of an interface it inherits from (a subclass may
        bind its own native version of an inherited member, like Element's nodeName)."""
        for anc in self.m.ancestors(iface):
            found = [x for x in self.m.interfaces[anc].members if x.name == name and x.kind in ("attribute", "operation") and not x.static]
            if found:
                return anc, found
        raise GenError(f"{iface}.{name} is not in the IDL")

    # -------------------------------------------------------- arguments
    def convert(self, a, i, op, iface):
        """Rust code for argument `i` (named `a.name`) and its Rust type."""
        t = self.m.resolve(a.type)
        ext = a.ext + t.ext
        v = f"conv::arg(args, {i})"
        name = t.name
        if t.union or a.variadic:
            raise GenError(f"{iface}.{op}: union and variadic arguments aren't supported yet ({t})")
        if name in STRINGS:
            if t.nullable:
                code, ty = f"conv::string_or_null(&{v}, ctx)?", "Option<String>"
            elif "LegacyNullToEmptyString" in ext:
                code, ty = f"conv::string_null_empty(&{v}, ctx)?", "String"
            else:
                code, ty = f"conv::string(&{v}, ctx)?", "String"
            default = None if a.default is None else (
                "None" if a.default == "null" else f"{a.default}.to_string()" if not t.nullable else f"Some({a.default}.to_string())")
        elif name == "boolean":
            code, ty = f"conv::boolean(&{v}, ctx)?", "bool"
            default = a.default
        elif name in INTS:
            bits, signed, ty = INTS[name]
            mode = "Enforce" if "EnforceRange" in ext else "Clamp" if "Clamp" in ext else "Wrap"
            code = f"conv::integer(&{v}, {bits}, {str(signed).lower()}, conv::IntMode::{mode}, ctx)? as {ty}"
            default = a.default
        elif name in DOUBLES:
            code, ty = f"conv::double(&{v}, {str(DOUBLES[name]).lower()}, ctx)?", "f64"
            default = None if a.default is None else str(float(a.default)) if a.default not in ("NaN", "Infinity", "-Infinity") else f"f64::{ {'NaN': 'NAN', 'Infinity': 'INFINITY', '-Infinity': 'NEG_INFINITY'}[a.default]}"
        elif name in self.m.enums:
            values = ", ".join(rust_str(x) for x in self.m.enums[name].values)
            code, ty = f"conv::enumeration(&{v}, &[{values}], {rust_str(name)}, ctx)?", "String"
            default = None if a.default is None else f"{a.default}.to_string()"
        elif self.is_node(t):
            code = f"conv::node(&{v}, {str(t.nullable).lower()}, {rust_str(op)}, {rust_str(iface)}, {i + 1}, ctx)?"
            ty = "Option<NodeId>"
            default = "None" if a.default == "null" else None
        else:
            raise GenError(f"{iface}.{op}: argument type {t} isn't supported yet")
        if t.nullable and name not in STRINGS and not self.is_node(t):
            raise GenError(f"{iface}.{op}: nullable {t} isn't supported yet")
        if a.optional:
            if default is not None:
                code = f"if conv::absent(args, {i}) {{ {default} }} else {{ {code} }}"
            elif ty.startswith("Option<"):
                code = f"if conv::absent(args, {i}) {{ None }} else {{ {code} }}"
            else:
                code = f"if conv::absent(args, {i}) {{ None }} else {{ Some({code}) }}"
                ty = f"Option<{ty}>"
        return code, ty

    def result(self, t):
        """The Rust type an implementation returns for IDL type `t` (for the docs)."""
        t = self.m.resolve(t)
        if t.name == "undefined":
            return "()"
        if t.name == "boolean":
            return "bool"
        if t.name in INTS:
            return INTS[t.name][2]
        if t.name in DOUBLES:
            return "f64"
        if t.name in STRINGS or t.name in self.m.enums:
            return "Option<String>" if t.nullable else "String"
        if self.is_node(t):
            return "Option<NodeId>"
        if t.name in NODE_LISTS or (t.name in ("sequence", "FrozenArray") and t.args and self.is_node(self.m.resolve(t.args[0]))):
            return "Vec<NodeId>"
        if t.name in ("sequence", "FrozenArray") and t.args and self.m.resolve(t.args[0]).name in STRINGS:
            return "Vec<String>"
        raise GenError(f"result type {t} isn't supported yet")

    # -------------------------------------------------------- members
    def member(self, iface, name, imp):
        """`imp` names the Rust function, or is {"fn": name, "throws": true} for one that
        returns Result<T, conv::DomError> (thrown as a DOMException)."""
        throws = isinstance(imp, dict) and imp.get("throws", False)
        imp = imp["fn"] if isinstance(imp, dict) else imp
        owner, found = self.find(iface, name)
        if len(found) > 1:
            raise GenError(f"{iface}.{name} is overloaded; overloads aren't supported yet")
        x = found[0]
        imp = imp if "::" in imp else f"dom::{imp}"
        getter = x.kind == "attribute"
        fname = f"{snake(iface)}_{snake(name)}" + ("_get" if getter else "")
        idl = f"attribute {x.type} {name}" if getter else f"{x.type} {name}(" + ", ".join(("optional " if a.optional else "") + f"{a.type} {a.name}" + (f" = {a.default}" if a.default is not None else "") for a in x.args) + ")"
        lines = [f"/// {owner}: `{idl}` -> `{imp}`" + (" (may throw)" if throws else ""),
                 f"fn {fname}(this: &JsValue, {'args' if x.args else '_'}: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {{",
                 "    let this = conv::this_node(this, ctx)?;"]
        required = 0
        if not getter:
            required = sum(1 for a in x.args if not a.optional and not a.variadic)
            if required:
                lines.append(f"    conv::require(args, {required}, {rust_str(name)}, {rust_str(iface)})?;")
            for i, a in enumerate(x.args):
                code, ty = self.convert(a, i, name, iface)
                lines.append(f"    let {var(a.name)}: {ty} = {code};")
        call_args = "".join(f", {var(a.name)}" for a in (x.args if not getter else []))
        if throws:
            lines.append(f"    let r: {self.result(x.type)} = {imp}(this{call_args}).map_err(|e| conv::dom_exception(e, ctx))?;")
        else:
            lines.append(f"    let r: {self.result(x.type)} = {imp}(this{call_args});")
        lines.append("    r.to_js(ctx)")
        lines.append("}")
        self.fns.append("\n".join(lines))
        return getter, name, fname, required

    def generate(self):
        tables = []
        for iface, spec in self.cfg["interfaces"].items():
            if iface not in self.m.interfaces:
                raise GenError(f"{iface} is not an interface in the IDL")
            if spec.get("this", "node") != "node":
                raise GenError(f"{iface}: only node interfaces are supported yet")
            if not self.m.inherits_from(iface, "Node"):
                raise GenError(f"{iface} is not a node interface")
            gets, ops = [], []
            for name, imp in spec["members"].items():
                getter, n, f, length = self.member(iface, name, imp)
                (gets if getter else ops).append(f"({rust_str(n)}, {f})" if getter else f"({rust_str(n)}, {f}, {length})")
            tables.append(
                f"    Interface {{\n        name: {rust_str(iface)},\n"
                f"        getters: &[{', '.join(gets)}],\n"
                f"        methods: &[{', '.join(ops)}],\n    }},")
        head = [
            "// GENERATED by tools/webidl/gen.py from the spec IDL in tools/webidl/idl/ and",
            "// tools/webidl/bindings.json. Don't edit: change those and run `python3 tools/webidl/gen.py`.",
            "",
            "#![allow(clippy::all, unused_variables)]",
            "",
            "use boa_engine::{Context, JsResult, JsValue};",
            "",
            "use super::conv::{self, ToJs};",
            "#[allow(unused_imports)]",
            "use super::dom;",
            "use super::Interface;",
            "#[allow(unused_imports)]",
            "use crate::dom::NodeId;",
            "",
        ]
        body = "\n\n".join(self.fns)
        table = "/// Every bound interface's native members.\npub const INTERFACES: &[Interface] = &[\n" + "\n".join(tables) + "\n];\n"
        return "\n".join(head) + "\n" + body + "\n\n" + table

    def report(self):
        rows = []
        for iface, spec in self.cfg["interfaces"].items():
            names = {x.name for x in self.m.interfaces[iface].members if x.kind in ("attribute", "operation") and x.name and not x.static}
            bound = set(spec["members"])
            rows.append((iface, len(bound), len(names), sorted(names - bound)))
        return rows


def load():
    cfg = json.load(open(os.path.join(HERE, "bindings.json")))
    files = [webidl.parse(open(os.path.join(HERE, "idl", f)).read()) for f in cfg["idl"]]
    return webidl.Model(files), cfg


def main():
    model, cfg = load()
    g = Gen(model, cfg)
    if "--report" in sys.argv:
        for iface, n, total, missing in g.report():
            print(f"{iface:20} {n:3} of {total:3} members native")
            print("    still JavaScript or missing: " + ", ".join(missing))
        return 0
    try:
        code = g.generate()
    except GenError as e:
        print(f"gen.py: {e}", file=sys.stderr)
        return 1
    if "--check" in sys.argv:
        have = open(OUT).read() if os.path.exists(OUT) else ""
        if have != code:
            print("src/bindings/generated.rs is out of date: run `python3 tools/webidl/gen.py`", file=sys.stderr)
            return 1
        return 0
    with open(OUT, "w") as f:
        f.write(code)
    print(f"wrote {os.path.relpath(OUT, ROOT)}: {sum(len(s['members']) for s in cfg['interfaces'].values())} members")
    return 0


if __name__ == "__main__":
    sys.exit(main())
