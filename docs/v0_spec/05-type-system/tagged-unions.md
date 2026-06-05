## 17. Tagged unions

### 17.1 Tagged tuple union arms

Inside `type [ ]`, a union item may be a **tagged tuple**: `(#atom, field : Type, ...)`. This is ordinary tuple type syntax whose first positional element is an atom singleton. The atom is interpreted as the implicit discriminant; named fields follow, separated by commas, using `:` for type annotation.

```zt
Shape :: type [
  (#circle, radius : Float);
  (#square, length : Float);
  (#rect, width : Float, height : Float);
]
```

This replaces the earlier pattern of embedding a `kind` discriminant field inside a record:

```zt
# Old style — no longer preferred
type [
  { kind : #circle; radius : Float; };
  { kind : #rect; width : Float; height : Float; };
]
```

The tagged tuple form is more concise and makes the discriminant structurally explicit without adding a separate variant grammar form.

### 17.2 Construction

A tagged tuple value mirrors the *shape* of the type, but binds each named field with `=`
(the same way value records use `=` while record types use `:`):

```zt
c := (#circle, radius = 5.0)
s := (#square, length = 10.0)
r := (#rect, width = 4.0, height = 3.0)
```

The atom must match one of the declared tagged tuple arms.

### 17.3 Pattern matching

Patterns in `match` expressions mirror the construction syntax:

```zt
area :: Shape -> Float
     :: (#circle, radius = r)          { r * r * 3.14159 }
     :: (#square, length = l)          { l * l }
     :: (#rect, width = w, height = h) { w * h }
```

Wildcard `_` ignores a field:

```zt
tag_of :: Shape -> Text
       :: (#circle, radius = _) { "circle" }
       :: (#square, length = _) { "square" }
       :: (#rect, width = _, height = _) { "rect" }
```

### 17.4 Nested tagged tuple patterns

Tagged tuple patterns can appear nested inside larger patterns:

```zt
Response :: type [
  (#ok, body : Shape);
  (#err, message : Text);
]

handle :: Response -> Float
       :: (#ok, body = (#circle, radius = r)) { r * r * 3.14159 }
       :: (#ok, body = _)                     { 0.0 }
       :: (#err, message = _)                 { 0.0 }
```

### 17.5 Desugaring

A tagged tuple union arm `(#circle, radius : Float)` desugars internally to a closed record type:

```zt
{ _tag : #circle; radius : Float; }
```

The `_tag` field is reserved and implicit. Users never write `_tag` directly; the compiler generates it. Pattern matching on a tagged tuple never requires matching `_tag` explicitly — the atom in the tuple's first position serves as the discriminant check.

### 17.6 Mixing atoms and tagged tuples

A union type may mix plain atom members and tagged tuple members:

```zt
Result :: type [
  (#ok, value : Int);
  #none;
]
```

Matching:

```zt
show :: Result -> Text
     :: (#ok, value = v) { "ok: " }
     :: #none            { "none" }
```

---
