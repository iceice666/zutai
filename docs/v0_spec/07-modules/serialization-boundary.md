## Serialization boundary

Serializable final values:

```zt
true
false
123
3.14
"hello"
#prod
[1; 2;]
{ host = "localhost"; }
```

Non-serializable final values:

```zt
\x. x
Type
Server
Text -> Text
(#ok, value = 1)
```

Type values are first-class compile-time values in `.zt`, but they do not have a direct `.zti` or JSON representation.

Tuple values are also general-mode values without a direct `.zti` representation in v0. Use records or lists when rendered output needs structured data.

This is valid as an imported module:

```zt
{
  Server = Server;
  normalize = normalize;
}
```

but invalid if rendered directly as `.zti` or JSON, because it contains a type and a function.

### Rendering atoms

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
