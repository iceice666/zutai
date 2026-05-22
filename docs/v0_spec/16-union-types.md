## 16. Union types

Union types use:

```zt
type [
  TypeExpr;
  TypeExpr;
]
```

Each item is a type-context expression: syntactically it may be any expression, but it must evaluate to a value of `Type`. In any type context, bare `{ ... }` and `[ ... ]` are interpreted as record and union type literals, so nested type literals do not need to repeat the `type` keyword.

Example:

```zt
let Profile: Type = type [
  #dev;
  #test;
  #prod;
]
```

In a type position, literals become singleton types.

So:

```zt
#dev
```

inside a type expression means:

> the type whose only value is `#dev`.

The same singleton-literal rule applies to `true`, `false`, and `none`:

```zt
let Enabled: Type = type [ true; false; ]
let MaybeProd: Type = type [ #prod; none; ]
```

These literals are singleton-capable, but only `#`-prefixed literals are atoms.

Therefore:

```zt
let Profile: Type = type [
  #dev;
  #test;
  #prod;
]
```

means a value of `Profile` must be exactly one of:

```zt
#dev
#test
#prod
```

Example:

```zt
let profile: Profile = #prod
```

Invalid:

```zt
let profile: Profile = #staging
```

because `#staging` is not a member of the union.

---

