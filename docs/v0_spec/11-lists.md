## 11. Lists

List types use the `List` type constructor:

```zt
List Text
List Int
List Server
```

This is the list type:

```zt
List Text
```

In ordinary expression context, this is a value-level list containing the type value `Text`:

```zt
[Text;]
```

This distinction keeps type-level list construction and value-level lists unambiguous. Inside type-context positions, `[ ... ]` is parsed as a union type literal instead; see [Union types](16-union-types.md).

---

