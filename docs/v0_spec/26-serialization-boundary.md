## 26. Serialization boundary

Serializable final values:

```zt
none
true
false
123
"hello"
#prod
[ ... ]
{ ... }
```

Non-serializable final values:

```zt
fn x => x
Type
Server
Text -> Text
```

This is valid as an imported module:

```zt
{
  Server = Server;
  normalize = normalize;
}
```

but invalid if rendered directly as `.zti` or JSON, because it contains a type and a function.

To render type information, use:

```zt
schema Server
```

not:

```zt
Server
```

### 26.1 Rendering atoms

When a `.zt` value containing atoms is rendered as `.zti`, the `#` prefix is preserved.

General-mode value:

```zt
{
  profile = #prod;
}
```

Rendered `.zti`:

```zti
{
  profile = #prod;
}
```

---

