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

### Local packages

A package is a directory containing an inert `zutai.zti` manifest. Manifests
are immediate-mode data rather than executable `.zt` modules because dependency
resolution must complete before general-mode name resolution, type checking, or
standard-library loading can run.

```zti
{
  formatVersion = 1;
  name = "app";
  compilerCompatibility = "0.1.0";
  modules = [];
  dependencies = [
    { alias = "math"; path = "../math"; };
  ];
}
```

A dependency package publishes modules explicitly in its own manifest:

```zti
{
  formatVersion = 1;
  name = "math";
  compilerCompatibility = "0.1.0";
  modules = [
    { name = "vector"; path = "src/vector.zt"; };
  ];
  dependencies = [];
}
```

The importing package addresses that module through the declared alias:

```zt
vector ::= import math.vector;
vector.length { 3; 4; }
```

The nearest ancestor `zutai.zti` owns a source file. Manifest format 1 accepts
package-relative local filesystem dependencies. Manifest format 2 accepts
explicit path and HTTPS Git source variants. A root package whose graph contains
a Git source owns a deterministic `zutai.lock.zti`; native `package sync`,
`fetch`, and `update` commands acquire and verify immutable content-addressed
snapshots. Checking, name resolution, LSP analysis, compilation, and browser
execution never acquire packages or rewrite the lock: they consume a validated
prepared graph and refuse stale locks, missing snapshots, unsafe paths, or
tampered sources. Registries, dependency-version solving, feature selection,
and executable build hooks are not implemented. Module paths must be
package-relative `.zt` paths and cannot escape their package root. Package
cycles, duplicate aliases/modules, incompatible compiler metadata, unknown
aliases, and unknown public modules are errors; identical package names at
different locked node identities may coexist transitively.

Each package resolves aliases from its own manifest, so dependencies may import
their own dependencies transitively without leaking those aliases into the root
package. Relative quoted imports retain their existing per-file subtree
confinement. Package sources and dependency metadata are carried in portable web
bundles, so filesystem and Wasm analysis resolve the same graph without Git,
filesystem, lockfile, or network access in the browser. See the
[package distribution decision](../../project/decisions.md#package-distribution-locked-git-source-snapshots).

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
`<stdlib-root>/zutai.zti`. Native frontends select that root from the global
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

`stdlib` is a reserved toolchain alias. The shipped distribution is physically
split into `base`, `data`, `system`, and `web` packages under the selected root,
while the root `zutai.zti` keeps every existing `stdlib.<name>` import stable.

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
