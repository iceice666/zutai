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
Profile :: type [
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
Enabled :: type [ true; false; ]
MaybeProd :: type [ #prod; none; ]
```

These literals are singleton-capable, but only `#`-prefixed literals are atoms.

Therefore:

```zt
Profile :: type [
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
profile : Profile = #prod
```

Invalid:

```zt
profile : Profile = #staging
```

because `#staging` is not a member of the union.

### 16.4 Open union types

Analogous to open record types, a union type may have an anonymous or named row tail to accept additional variants.

An anonymous union tail:

```zt
type [
  #dev;
  #test;
  ...;
]
```

means any union that includes at least `#dev` and `#test` as members.

A named union tail preserves the extra variants in the result type:

```zt
type [
  #dev;
  #test;
  ...Rest;
]
```

Example:

```zt
handle_env :: forall Rest. [ #dev; #test; ...Rest; ] -> Text -> Text
           :: #dev -> msg { "dev: " }
           :: #test -> msg { "test: " }
           :: _ -> msg { msg }
```

Union tails also work with variant constructors:

```zt
type [
  (#circle, radius : Float);
  ...Rest;
]
```

Union extension — extend an existing union with new variants:

```zt
Shape3D :: type [
  ...Shape;
  (#sphere, radius : Float);
]
```

---
