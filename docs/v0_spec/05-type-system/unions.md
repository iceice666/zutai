## Union types

Union types use `type [ ... ]`. There are two forms.

### Pure enum

When all members are bare tag names with no payload:

```zt
Profile :: type [dev; test; prod;]

Dir :: type [north; east; west; south;]
```

Tag names do not carry `#` inside the definition. At use sites, values are ordinary atoms:

```zt
profile : Profile = #prod
```

### Tagged union

When some or all members carry a record payload, use semicolon-terminated members:

```zt
Shape :: type [
  circle: { radius: Float; };
  square: { length: Float; };
  rect:   { width: Float; height: Float; };
]
```

Each member is either a bare singleton tag or a `name: { ... }` pair. Members may be mixed:

```zt
Result :: type [
  ok: { value: Int; };
  none;
]
```

### Construction

Singleton tags are bare atoms:

```zt
d := #north
```

Tags with payloads are an atom followed by a record:

```zt
c := #circle { radius = 5.0; }
s := #square { length = 10.0; }
r := #rect   { width = 4.0; height = 3.0; }
```

### The `.tag` accessor

Every tagged union value exposes a `.tag` field that returns the atom tag:

```zt
c.tag   -- evaluates to #circle
s.tag   -- evaluates to #square
```

### Pattern matching

Singleton tags match as atoms:

```zt
match profile {
  | #dev  => false;
  | #test => false;
  | #prod => true;
}
```

Tags with payloads match with a record destructure:

```zt
area :: Shape -> Float {
  | #circle { radius = r }           => r * r * 3.14159;
  | #square { length = l }           => l * l;
  | #rect   { width = w; height = h } => w * h;
}
```

For finite union types, `match` must be exhaustive over all tags. See
[Pattern matching](../06-polymorphism/pattern-matching.md).

### Generic union types

Union types may be generic:

```zt
Optional :: <T> type [
  none;
  some: { value: T; };
]
```

`T?` is shorthand for `Optional T`. See [Optional values](optional-values.md).

---

Open union types and union extension are v1 features. See [Row polymorphism](../../v1_spec/01-row-polymorphism.md).
