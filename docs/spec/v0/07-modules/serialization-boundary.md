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

Non-serializable final values (no `.zti` or JSON representation):

```zt
\x. x
Type
Server
Text -> Text
```

Type values and functions are first-class compile-time or runtime values in `.zt`, but they do not have a direct `.zti` or JSON representation.

Tagged union values with payloads are **not** directly representable in `.zti`, but they **are** serializable to JSON when evaluated via `eval_path_to_json`. See [JSON rendering](#rendering-to-json) below.

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

### Rendering to JSON

`eval_path_to_json` serializes tagged union values using a two-key envelope. The rules are:

| Zutai value | JSON shape |
| --- | --- |
| bare atom `#tag` | string `"#tag"` (the `#` prefix is preserved) |
| `#tag { field = v; ... }` (record payload) | `{"tag": "tag", "payload": {"field": v, ...}}` (no `#` on the tag key value) |
| `#tag (a, b)` (tuple payload) | `{"tag": "tag", "payload": [a, b]}` |

Example:

```zt
Action :: type { #quit; #spawn: { command : Text; }; }

{
  a = #quit;
  b = #spawn { command = "ghostty"; };
}
```

Renders as JSON:

```json
{
  "a": "#quit",
  "b": { "tag": "spawn", "payload": { "command": "ghostty" } }
}
```

Consumers must handle both shapes. The tag string in the `"tag"` key does **not** include `#`; only the bare-atom string form includes `#`.

---
