## 16. Union types

Union types use:

```zt
type [
  TypeExpr;
  TypeExpr;
]
```

Example:

```zt
Profile :: type [
  #dev;
  #test;
  #prod;
]
```

In a type position, atom literals become singleton types.

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

---

Open union types and union extension are v1 features. See [Row polymorphism](../../v1_spec/01-row-polymorphism.md).
