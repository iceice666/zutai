## Imports

Imports are top-level declarations.

Canonical form:

```zt
cfg :: import "config.zti"
lib :: import "server.zt"
```

The declaration creates one binding named by the declaration. Imports are prefixed only: imported values are used as `cfg` or `lib.field`, and imported type-valued fields are used in annotations as `lib.Type`.

A shorthand unquoted import path may be accepted by implementations:

```zt
cfg :: import config.zti
```

However, the canonical syntax is the string form.

`import` is not an expression. Code such as `cfg := import "config.zti"` is rejected. Runtime-selected or dynamic `.zti` loading is not `import`; it belongs to a later explicit effect/capability design.

### Importing `.zti`

```zt
cfg :: import "file.zti"
```

parses the `.zti` file and binds the corresponding `.zt` data value to `cfg`. Blocks become records and arrays become lists. No `.zti` expression is evaluated, because immediate mode has only values.

A `.zti` atom such as:

```zti
#prod
```

is represented in `.zt` with the same atom spelling:

```zt
#prod
```

### Importing `.zt`

```zt
lib :: import "file.zt"
```

evaluates the imported `.zt` file and binds its final expression to `lib`.

The returned value may contain records, lists, functions, or types. If a returned record contains a type-valued field, consumers can use it in annotations through the import prefix:

```zt
server :: lib.Server = {
  host = "localhost";
  port = lib.defaultPort;
}
```

### Import purity

Imports are:

* pure
* deterministic
* path-relative
* cached

Re-importing the same resolved file path returns the same value.

### Path confinement

Import paths are confined to the importing file's directory subtree.

* Absolute paths (e.g. `"/tmp/x.zti"`) are always rejected.
* Relative paths are resolved against the importing file's directory and the
  resolved canonical path must remain inside that directory.  A path such as
  `"../../../etc/foo.zti"` that escapes the subtree is rejected with a
  `PathTraversal` diagnostic even if the target file exists on disk.
* Symlinks are fully resolved before the containment check, so a symlink that
  points outside the base directory is also rejected.

---
