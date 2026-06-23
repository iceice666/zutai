## Laziness and purity

General mode is pure and lazy.

Bindings are immutable:

```zt
expensive ::= computeHugeThing cfg

{
  name = cfg.name;
}
```

If `expensive` is never demanded, it is never evaluated.

Function arguments are lazy by default unless forced.

Imports are pure.

No implicit ambient effects exist in the core language.

The following should not exist as ambient primitives in the pure core:

```zt
now()
random()
readFile "/tmp/a"
shell "git status"
env "HOME"
```

External data enters through explicit static import declarations, usually as `.zti`:

```zt
env  :: import "env.zti"
args :: import "args.zti"
cfg  :: import "config.zti"
```

Runtime-selected `.zti` loading is not `import`; it belongs to a later explicit effect/capability design.

---
