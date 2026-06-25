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
serverLib :: import "server.zt";
raw :: import "server.zti";

server :: serverLib.Server = serverLib.normalize raw;

server
```

This works because imported `.zt` modules can contain non-serializable values like functions and types, and import declarations expose them through the chosen prefix.

Only rendering requires serializability.

### Standard-library imports

A dotted import resolves against the embedded standard library instead of the
filesystem:

```zt
s :: import stdlib.stream;

s.map f (s.singleton 1)
```

`import stdlib.<name>` needs no install path and no file next to the program;
the module source is built into the compiler. Resolution does not consult the
filesystem and is not subject to the path-relative subtree confinement that
quoted-string imports use. An unknown `<name>` is a precise diagnostic
(`unknown stdlib module: stdlib.<name>`). Currently `stdlib.stream` is provided.

### Selective binding (destructuring import)

An import binds one name and its members are reached by field access (`s.map`).
A destructuring binding brings selected members into scope **unqualified**,
reusing the select-field list syntax on the left of `::=`:

```zt
s :: import stdlib.stream;
{ map; fold; singleton; } ::= s;

fold (+) 0 (map double (singleton 1))
```

The right-hand side is any record-valued expression, so it composes with the
`>>=` select operator (`{ map; } ::= s >>= { map; };`) and with a prior import
binding. The receiver is evaluated once. A name that is not a field of the
record is a type error; a destructured name that collides with another top-level
binding is a duplicate-binding error.

Type-valued members (e.g. `Stream`) may be exported and selected, but a
parametric imported type constructor cannot yet be *applied* in an annotation
(`x : s.Stream Int`); inference flows structurally without the annotation.

---
