## Union types

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

The same singleton-literal rule applies to `true` and `false`:

```zt
Enabled :: type [ true; false; ]
MaybeProd :: type [ #prod; #none; ]
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

### Tuple members

Union members may be tuple types. This is the ordinary representation for
structured alternatives:

```zt
Shape :: type [
  (#circle, radius : Float);
  (#square, length : Float);
  (#rect, width : Float, height : Float);
]
```

Each member above is a tuple type. Its first item is an atom singleton type, and
the remaining named items are tuple fields. There is no separate union form and
no hidden field.

A value mirrors the tuple shape, but binds named fields with `=`:

```zt
c := (#circle, radius = 5.0)
s := (#square, length = 10.0)
r := (#rect, width = 4.0, height = 3.0)
```

The atom is just the first tuple value. It must match the corresponding
singleton type for the tuple member.

Tuple alternatives are normal `.zt` tuple values. They can be imported or exported by `.zt` modules, matched, and compared structurally when their fields are comparable. v0 does not define a direct `.zti` serialization for tuple values.

Pattern matching uses the same tuple shape:

```zt
area :: Shape -> Float {
  | (#circle, radius = r)          => r * r * 3.14159;
  | (#square, length = l)          => l * l;
  | (#rect, width = w, height = h) => w * h;
}
```

Tuple members can be nested:

```zt
Response :: type [
  (#ok, body : Shape);
  (#err, message : Text);
]

handle :: Response -> Float {
  | (#ok, body = (#circle, radius = r)) => r * r * 3.14159;
  | (#ok, body = _)                     => 0.0;
  | (#err, message = _)                 => 0.0;
}
```

Union types may freely mix singleton members and tuple members:

```zt
Result :: type [
  (#ok, value : Int);
  #none;
]
```

---

Open union types and union extension are v1 features. See [Row polymorphism](../../v1_spec/01-row-polymorphism.md).
