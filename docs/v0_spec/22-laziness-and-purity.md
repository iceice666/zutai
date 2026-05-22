## 22. Laziness and purity

General mode is pure and lazy.

Bindings are immutable:

```zt
let expensive = computeHugeThing cfg

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

External data enters through explicit imports, usually as `.zti`:

```zt
let env = import "env.zti"
let args = import "args.zti"
let cfg = import "config.zti"
```

---

