## Union types

Union types use `type { ... }`. Members are semicolon-terminated tags with
optional payloads.

### Pure enum

When all members are bare tag names with no payload:

```zt
Profile :: type {#dev; #test; #prod;};

Dir :: type {#north; #east; #west; #south;};
```

Union member tags are written with `#` inside the definition. At use sites, values use the same atom spelling:

```zt
profile :: Profile = #prod;
```

### Tagged union

Members may carry either a record payload or a positional tuple payload.
Record payloads use `#tag: { ... }`:

```zt
Shape :: type {
  #circle: { radius: Float; };
  #square: { length: Float; };
  #rect: { width: Float; height: Float; };
};
```

Positional payload variants use `#tag: (...)`:

```zt
Message :: type {
  #quit;
  #move: (Int, Int);
  #write: (Text);
};
```

Members may be mixed:

```zt
Result :: type {
  #ok: { value: Int; };
  #err: (Text);
  #none;
};
```

### Construction

Singleton tags are bare atoms:

```zt
d ::= #north;
```

Record payload values are an atom followed by a record:

```zt
c ::= #circle { radius = 5.0; };
s ::= #square { length = 10.0; };
r ::= #rect   { width = 4.0; height = 3.0; };
```

Positional payload values are an atom followed by a tuple payload:

```zt
m ::= #move (10, 20);
w ::= #write ("hello");
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
area :: Shape -> Float
  = #circle { radius = r; }           => r * r * 3.14159;
  = #square { length = l; }           => l * l;
  = #rect   { width = w; height = h; } => w * h;
```

Positional payloads match with a tuple destructure:

```zt
describe :: Message -> Text
  = #quit => "quit";
  = #move (x, y) => "move";
  = #write (text) => text;
```

For finite union types, `match` must be exhaustive over all tags. See
[Pattern matching](../06-polymorphism/pattern-matching.md).

### Generic union types

Union types may be generic:

```zt
Optional :: <T> type {
  #none;
  #some (T);
};
```

`T?` is shorthand for `Optional T`. See [Optional values](optional-values.md).

---

### JSON serialization

When a tagged union value is serialized to JSON via `eval_path_to_json`, the shape depends on whether the variant carries a payload:

| Zutai value | JSON |
| --- | --- |
| bare atom `#tag` | `"#tag"` — a JSON string with the `#` prefix preserved |
| `#tag { field = v; }` | `{"tag": "tag", "payload": {"field": v}}` — the tag name has **no** `#` prefix in the object |
| `#tag (a, b)` | `{"tag": "tag", "payload": [a, b]}` |

A consumer therefore sees two distinct shapes and must branch on whether the value is a string or an object. Example in Rust with `serde_json`:

```rust
#[serde(untagged)]
enum RawAction {
    Atom(String),                                      // "#quit", "#toggle_pin", …
    Tagged { tag: String, payload: serde_json::Value }, // tag = "spawn" (no #)
}
```

Tagged union values have no `.zti` representation; only JSON rendering is supported.

---

Open union types and union extension are v1 features. See [Row polymorphism](../../v1/01-row-polymorphism.md).
