## Modules

A `.zt` file can return a record containing values, functions, and types.

Example `server.zt`:

```zt
RawServer :: type {
  host? : Text;
  port? : Int;
  tls? : Bool;
};

Server :: type {
  host : Text;
  port : Int;
  tls : Bool;
};

normalize :: RawServer -> Server
  = raw => {
    host = raw.host ?? "127.0.0.1";
    port = raw.port ?? 8080;
    tls = raw.tls ?? false;
  };

{
  RawServer = RawServer;
  Server = Server;
  normalize = normalize;
}
```

Another file:

```zt
serverLib ::= import "server.zt";
raw ::= import "server.zti";

server :: serverLib.Server = serverLib.normalize raw;

server
```

This works because imported `.zt` modules can contain non-serializable values like functions and types, and an import binding exposes them through the chosen prefix.

Only rendering requires serializability.

### Standard-library imports

A dotted import resolves against the configured filesystem standard library,
not relative to the importing module:

```zt
s ::= import stdlib.stream;

s.map f (s.singleton 1)
```

Several static module aliases are ordinary import bindings; imported members
remain prefixed:

```zt
s ::= import stdlib.stream;
n ::= import stdlib.num;
t ::= import stdlib.text;

s.map f (s.singleton 1)
```

`import stdlib.<name>` reads a module named by the version-checked
`<stdlib-root>/manifest.json`. Native frontends select that root from the global
`--stdlib-root` option, then `ZUTAI_STDLIB_ROOT`, then the compiler-relative
`../share/zutai/stdlib` installation. Stdlib resolution is not subject to the
path-relative subtree confinement that quoted-string imports use. A missing,
incompatible, or malformed library is a setup error; an unknown `<name>` is the
precise diagnostic `unknown stdlib module: stdlib.<name>`. Currently provided modules
are `stdlib.stream`, `stdlib.prelude`, `stdlib.optional`, `stdlib.result`,
`stdlib.num`, `stdlib.text`, `stdlib.cmp`, `stdlib.config`,
`stdlib.reflect`, `stdlib.list`, `stdlib.data`, `stdlib.validate`, `stdlib.fs`,
`stdlib.net`, `stdlib.css`, `stdlib.html`, and `stdlib.browser`.

Portable web bundles carry the exact ambient and transitively imported stdlib
sources selected by the native builder. The Wasm kernel resolves those sources
from the bundle and performs no filesystem or network lookup.

### Selective binding (destructuring import)

An import binds one name and its members are reached by field access (`s.map`).
A destructuring binding brings selected members into scope **unqualified**,
reusing the select-field list syntax on the left of `::=`. Because `import` is an
expression, members can be destructured straight off the import in one binding:

```zt
{ map; fold; singleton; } ::= import stdlib.stream;

fold (+) 0 (map double (singleton 1))
```

The right-hand side is any record-valued expression, so it equally composes with a
prior import binding (`s ::= import stdlib.stream; { map; fold; } ::= s;`) and with
the `>>=` select operator (`{ map; } ::= s >>= { map; };`). The receiver is
evaluated once. A name that is not a field of the record is a type error; a
destructured name that collides with another top-level binding is a
duplicate-binding error.

Type-valued members (e.g. `Stream`) may be exported, selected, and **applied** in
an annotation: a parametric imported type constructor resolves through qualified
access, so `xs :: s.Stream Int = s.fromList {1; 2; 3;}` type-checks and the value
built by imported combinators (`s.fromList`, `s.cons`) unifies with `s.Stream Int`.
A constructor with a higher-kinded parameter (`<F :: Type -> Type>`) cannot cross
the import boundary and is refused; a bare constructor used without arguments
(`x :: s.Stream`) is an arity error, exactly like a local generic alias.

---
