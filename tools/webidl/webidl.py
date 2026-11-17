"""A WebIDL parser (https://webidl.spec.whatwg.org/), enough for the spec IDL that w3c/webref
extracts from the standards. No dependencies.

    defs = parse(text)          # list of Definition
    model = Model([defs, ...])  # interfaces with partials and mixins merged, inheritance

Types come back as Type(name, nullable, args, union) where `args` holds generic parameters
(sequence<Node> -> Type("sequence", args=[Type("Node")])) and `union` the members of
(A or B). Extended attributes are kept as raw strings ("CEReactions", "LegacyNullToEmptyString").
"""
import re
from dataclasses import dataclass, field

TOKEN = re.compile(
    r"""
    (?P<ws>\s+|//[^\n]*|/\*.*?\*/)
  | (?P<number>-?(?:0[Xx][0-9A-Fa-f]+|(?:[0-9]+\.[0-9]*|[0-9]*\.[0-9]+)(?:[Ee][+-]?[0-9]+)?|[0-9]+[Ee][+-]?[0-9]+|0[0-7]*|[1-9][0-9]*))
  | (?P<string>"[^"]*")
  | (?P<ident>[_-]?[A-Za-z][0-9A-Z_a-z-]*)
  | (?P<other>\.\.\.|[^\s0-9A-Za-z])
    """,
    re.S | re.X,
)


class IdlError(Exception):
    pass


def tokenize(text):
    out, pos = [], 0
    while pos < len(text):
        m = TOKEN.match(text, pos)
        if not m:
            raise IdlError(f"bad character at {pos}: {text[pos:pos + 20]!r}")
        pos = m.end()
        if m.lastgroup != "ws":
            out.append((m.lastgroup, m.group()))
    return out


@dataclass
class Type:
    name: str
    nullable: bool = False
    args: list = field(default_factory=list)
    union: list = field(default_factory=list)
    ext: list = field(default_factory=list)

    def __str__(self):
        if self.union:
            s = "(" + " or ".join(map(str, self.union)) + ")"
        elif self.args:
            s = f"{self.name}<{', '.join(map(str, self.args))}>"
        else:
            s = self.name
        return s + ("?" if self.nullable else "")


@dataclass
class Argument:
    name: str
    type: Type
    optional: bool = False
    variadic: bool = False
    default: object = None  # the default's source text, e.g. "false", "{}", '"x"'
    ext: list = field(default_factory=list)


@dataclass
class Member:
    kind: str  # attribute, operation, const, constructor, iterable, maplike, setlike, async_iterable, stringifier
    name: str = ""
    type: Type = None
    args: list = field(default_factory=list)
    readonly: bool = False
    static: bool = False
    special: str = ""  # getter, setter, deleter, stringifier, inherit
    value: str = ""
    ext: list = field(default_factory=list)
    origin: str = ""  # the interface or mixin that declared it


@dataclass
class Definition:
    kind: str  # interface, mixin, dictionary, enum, typedef, includes, callback, callback_interface, namespace
    name: str
    partial: bool = False
    inherits: str = ""
    members: list = field(default_factory=list)
    values: list = field(default_factory=list)  # enum values
    type: Type = None  # typedef / callback return
    target: str = ""  # includes: the mixin
    ext: list = field(default_factory=list)


class Parser:
    def __init__(self, text):
        self.t = tokenize(text)
        self.i = 0

    def peek(self, k=0):
        j = self.i + k
        return self.t[j][1] if j < len(self.t) else None

    def take(self, want=None):
        if self.i >= len(self.t):
            raise IdlError(f"unexpected end, wanted {want}")
        v = self.t[self.i][1]
        if want is not None and v != want:
            raise IdlError(f"wanted {want!r}, got {v!r} near {' '.join(x[1] for x in self.t[self.i - 5:self.i + 5])}")
        self.i += 1
        return v

    def opt(self, v):
        if self.peek() == v:
            self.i += 1
            return True
        return False

    def ident(self):
        kind, v = self.t[self.i]
        if kind != "ident":
            raise IdlError(f"wanted a name, got {v!r}")
        self.i += 1
        return v.lstrip("_")

    # [A, B=c, D(args), E=(x, y)] -> ["A", "B=c", "D(...)", "E=(x,y)"]
    def ext_attrs(self):
        if self.peek() != "[":
            return []
        self.take("[")
        out, cur, depth = [], [], 0
        while True:
            v = self.take()
            if v in "([{":
                depth += 1
            elif v in ")]}":
                if v == "]" and depth == 0:
                    break
                depth -= 1
            if v == "," and depth == 0:
                out.append("".join(cur))
                cur = []
            else:
                cur.append(v)
        if cur:
            out.append("".join(cur))
        return out

    def type(self):
        ext = self.ext_attrs()
        if self.peek() == "(":
            self.take("(")
            members = [self.type()]
            while self.opt("or"):
                members.append(self.type())
            self.take(")")
            t = Type("union", union=members)
        else:
            words = []
            first = self.take()
            if first in ("unsigned", "unrestricted"):
                words.append(first)
                first = self.take()
            words.append(first)
            if first == "long" and self.peek() == "long":
                words.append(self.take())
            t = Type(" ".join(words))
            if self.peek() == "<":
                self.take("<")
                t.args.append(self.type())
                while self.opt(","):
                    t.args.append(self.type())
                self.take(">")
        if self.opt("?"):
            t.nullable = True
        t.ext = ext
        return t

    def default_value(self):
        v = self.take()
        if v == "[":
            self.take("]")
            return "[]"
        if v == "{":
            self.take("}")
            return "{}"
        if v == "-" and self.peek() == "Infinity":
            return "-" + self.take()
        return v

    def arguments(self):
        self.take("(")
        out = []
        while self.peek() != ")":
            ext = self.ext_attrs()
            optional = self.opt("optional")
            t = self.type()
            variadic = self.opt("...")
            name = self.take().lstrip("_")
            a = Argument(name, t, optional, variadic, ext=ext)
            if self.opt("="):
                a.default = self.default_value()
            out.append(a)
            if not self.opt(","):
                break
        self.take(")")
        return out

    def member(self, owner):
        ext = self.ext_attrs()
        p = self.peek()
        if p == "const":
            self.take()
            t = self.type()
            name = self.ident()
            self.take("=")
            value = self.default_value()
            self.take(";")
            return Member("const", name, t, value=value, ext=ext, origin=owner)
        if p == "constructor":
            self.take()
            args = self.arguments()
            self.take(";")
            return Member("constructor", "constructor", args=args, ext=ext, origin=owner)
        if p in ("iterable", "maplike", "setlike", "async_iterable") or (p == "async" and self.peek(1) in ("iterable", "maplike", "setlike")) or (p == "readonly" and self.peek(1) in ("maplike", "setlike")):
            readonly = self.opt("readonly")
            kind = self.take()
            if kind == "async":
                kind = "async_" + self.take()
            self.take("<")
            args = [self.type()]
            while self.opt(","):
                args.append(self.type())
            self.take(">")
            if self.peek() == "(":
                self.arguments()
            self.take(";")
            return Member(kind, kind, Type(kind, args=args), readonly=readonly, ext=ext, origin=owner)
        static = special = ""
        if p == "static":
            self.take()
            static = True
        if self.peek() in ("getter", "setter", "deleter", "stringifier", "inherit"):
            special = self.take()
            if special == "stringifier" and self.peek() == ";":
                self.take(";")
                return Member("stringifier", "toString", ext=ext, origin=owner)
        readonly = self.opt("readonly")
        if self.peek() == "attribute":
            self.take()
            t = self.type()
            name = self.take().lstrip("_")
            self.take(";")
            return Member("attribute", name, t, readonly=readonly, static=bool(static), special=special, ext=ext, origin=owner)
        t = self.type()
        name = ""
        if self.peek() != "(":
            name = self.take().lstrip("_")
        args = self.arguments()
        self.take(";")
        return Member("operation", name, t, args=args, static=bool(static), special=special, ext=ext, origin=owner)

    def dictionary_member(self, owner):
        ext = self.ext_attrs()
        required = self.opt("required")
        t = self.type()
        name = self.take().lstrip("_")
        m = Member("field", name, t, ext=ext, origin=owner, readonly=required)
        if self.opt("="):
            m.value = self.default_value()
        self.take(";")
        return m

    def body(self, d, member):
        self.take("{")
        while not self.opt("}"):
            d.members.append(member(d.name))
        self.take(";")

    def definition(self):
        ext = self.ext_attrs()
        partial = self.opt("partial")
        k = self.take()
        if k == "callback":
            if self.opt("interface"):
                d = Definition("callback_interface", self.ident(), ext=ext)
                self.body(d, self.member)
                return d
            name = self.ident()
            self.take("=")
            d = Definition("callback", name, type=self.type(), ext=ext)
            d.members = [Member("operation", "", d.type, args=self.arguments())]
            self.take(";")
            return d
        if k in ("interface", "namespace"):
            mixin = k == "interface" and self.opt("mixin")
            d = Definition("mixin" if mixin else k, self.ident(), partial=partial, ext=ext)
            if self.opt(":"):
                d.inherits = self.ident()
            self.body(d, self.member)
            return d
        if k == "dictionary":
            d = Definition("dictionary", self.ident(), partial=partial, ext=ext)
            if self.opt(":"):
                d.inherits = self.ident()
            self.body(d, self.dictionary_member)
            return d
        if k == "enum":
            d = Definition("enum", self.ident(), ext=ext)
            self.take("{")
            while not self.opt("}"):
                d.values.append(self.take().strip('"'))
                self.opt(",")
            self.take(";")
            return d
        if k == "typedef":
            t = self.type()
            d = Definition("typedef", self.ident(), type=t, ext=ext)
            self.take(";")
            return d
        # "A includes B;"
        name = k.lstrip("_")
        self.take("includes")
        d = Definition("includes", name, target=self.ident(), ext=ext)
        self.take(";")
        return d

    def parse(self):
        out = []
        while self.i < len(self.t):
            out.append(self.definition())
        return out


def parse(text):
    return Parser(text).parse()


class Model:
    """Every definition from a set of IDL files, with partial interfaces folded in and
    mixins expanded into the interfaces that include them."""

    def __init__(self, files):
        self.interfaces, self.mixins, self.dictionaries, self.enums, self.typedefs = {}, {}, {}, {}, {}
        partials, includes = [], []
        for defs in files:
            for d in defs:
                if d.kind == "includes":
                    includes.append(d)
                elif d.partial:
                    partials.append(d)
                elif d.kind in ("interface", "namespace", "callback_interface"):
                    self.interfaces[d.name] = d
                elif d.kind == "mixin":
                    self.mixins[d.name] = d
                elif d.kind == "dictionary":
                    self.dictionaries[d.name] = d
                elif d.kind == "enum":
                    self.enums[d.name] = d
                elif d.kind == "typedef":
                    self.typedefs[d.name] = d.type
        for p in partials:
            table = {"mixin": self.mixins, "dictionary": self.dictionaries}.get(p.kind, self.interfaces)
            if p.name in table:
                table[p.name].members.extend(p.members)
        for inc in includes:
            i, m = self.interfaces.get(inc.name), self.mixins.get(inc.target)
            if i and m:
                i.members.extend(m.members)

    def ancestors(self, name):
        """The interface and everything it inherits from, nearest first."""
        out = []
        while name in self.interfaces:
            out.append(name)
            name = self.interfaces[name].inherits
        return out

    def inherits_from(self, name, base):
        return base in self.ancestors(name)

    def resolve(self, t):
        """Follow typedefs (keeping nullability)."""
        seen = 0
        while t.name in self.typedefs and not t.union and seen < 20:
            u = self.typedefs[t.name]
            t = Type(u.name, t.nullable or u.nullable, u.args, u.union, u.ext + t.ext)
            seen += 1
        return t
