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

### 11.1 Standard list operations

The core standard-library list functions:

```zt
map    :: forall A B. (A -> B) -> List A -> List B
filter :: forall A. (A -> Bool) -> List A -> List A
fold   :: forall A B. (B -> A -> B) -> B -> List A -> B
zip    :: forall A B. List A -> List B -> List (A, B)
flatten :: forall A. List (List A) -> List A
```

Usage example using `|>`:

```zt
items
  |> filter (\x => x > 0)
  |> map (\x => x * 2)
  |> fold (\acc x => acc + x) 0
```

---
