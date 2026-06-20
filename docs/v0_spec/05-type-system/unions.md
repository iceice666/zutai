## Union types

Union types use `type { ... }`. Members are semicolon-terminated tags with
optional payloads.

### Pure enum

When all members are bare tag names with no payload:

```zt
Profile :: type {#dev; #test; #prod;}

Dir :: type {#north; #east; #west; #south;}
```

Union member tags are written with `#` inside the definition. At use sites, values use the same atom spelling:

```zt
profile :: Profile = #prod
```

### Tagged union

Members may carry either a record payload or a positional tuple payload.
Record payloads use `#tag: { ... }`:

```zt
Shape :: type {
  #circle: { radius: Float; };
  #square: { length: Float; };
  #rect: { width: Float; height: Float; };
}
```

Positional payload variants use `#tag: (...)`:

```zt
Message :: type {
  #quit;
  #move: (Int, Int);
  #write: (Text);
}
```

Members may be mixed:

```zt
Result :: type {
  #ok: { value: Int; };
  #err: (Text);
  #none;
}
```

### Construction

Singleton tags are bare atoms:

```zt
d := #north
```

Record payload values are an atom followed by a record:

```zt
c := #circle { radius = 5.0; }
s := #square { length = 10.0; }
r := #rect   { width = 4.0; height = 3.0; }
```

Positional payload values are an atom followed by a tuple payload:

```zt
m := #move (10, 20)
w := #write ("hello")
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

Record payloads match with a record destructure:

```zt
area :: Shape -> Float {
  | #circle { radius = r; }           => r * r * 3.14159;
  | #square { length = l; }           => l * l;
  | #rect   { width = w; height = h; } => w * h;
}
```

Positional payloads match with a tuple destructure:

```zt
describe :: Message -> Text {
  | #quit => "quit";
  | #move (x, y) => "move";
  | #write (text) => text;
}
```

For finite union types, `match` must be exhaustive over all tags. See
[Pattern matching](../06-polymorphism/pattern-matching.md).

### Generic union types

Union types may be generic:

```zt
Optional :: <T> type {
  #none;
  #some: { value: T; };
}
```

`T?` is shorthand for `Optional T`. See [Optional values](optional-values.md).

---

Open union types and union extension are v1 features. See [Row polymorphism](../../v1_spec/01-row-polymorphism.md).
