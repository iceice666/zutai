## 17. Tuple variants

### 17.1 Tuple union members

Inside `type [ ]`, a union item may be a tuple type:

```zt
Shape :: type [
  (#circle, radius : Float);
  (#square, length : Float);
  (#rect, width : Float, height : Float);
]
```

This is ordinary tuple type syntax. Atom literals in tuple type positions are singleton types, so `#circle` means the type whose only value is `#circle`. Named tuple fields use `:` in type position.

This replaces the earlier pattern of embedding a `kind` field inside a record:

```zt
# Old style — no longer preferred
type [
  { kind : #circle; radius : Float; };
  { kind : #rect; width : Float; height : Float; };
]
```

The tuple form is more concise and makes the variant shape structural without adding a separate variant grammar form.

### 17.2 Construction

A tuple variant value mirrors the shape of the type, but binds each named field with `=`
(the same way value records use `=` while record types use `:`):

```zt
c := (#circle, radius = 5.0)
s := (#square, length = 10.0)
r := (#rect, width = 4.0, height = 3.0)
```

The tuple value must match one of the declared union member shapes.

Multiple atom singleton items are allowed:

```zt
Job :: type [
  (#builder, #macos, prompt : Text);
  (#builder, #linux, prompt : Text);
  (#tester, #macos, suite : Text);
]

job := (#builder, #macos, prompt = "compile")
```

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

Tuple variants with multiple atom singleton items are matched by the same tuple shape:

```zt
run :: Job -> Text
    :: (#builder, #macos, prompt = p) { p }
    :: (#builder, #linux, prompt = p) { p }
    :: (#tester, #macos, suite = s)   { s }
```

### 17.4 Nested tuple variant patterns

Tuple patterns can appear nested inside larger patterns:

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

### 17.5 No hidden tag field

Tuple variants do not desugar to records and do not introduce hidden fields. A tuple union member such as:

```zt
(#circle, radius : Float)
```

is a tuple type whose first positional item is the singleton atom type `#circle` and whose second item is the named field `radius : Float`.

The name `_tag` has no special meaning in v0. It may be used like any other non-keyword identifier or field name.

### 17.6 Mixing atoms and tuple variants

A union type may mix plain atom members and tuple members:

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
