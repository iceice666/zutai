## 11. Lists

List types use the `List` type constructor:

```zt
List Text
List Int
List Server
```

List values use `[` `;` `]`:

```zt
items : List Int = [1; 2; 3;]
```

### 11.1 Standard list operations

The core standard-library list functions:

```zt
map    :: [A, B] (A -> B) -> List A -> List B
filter :: [A] (A -> Bool) -> List A -> List A
fold   :: [A, B] (B -> A -> B) -> B -> List A -> B
zip    :: [A, B] List A -> List B -> List (A, B)
flatten :: [A] List (List A) -> List A
```

Usage example using `|>`:

```zt
items
  |> filter (\x => x > 0)
  |> map (\x => x * 2)
  |> fold (\acc x => acc + x) 0
```

---
