## Equality

Structural equality is defined for first-order data values:

* `Bool`
* `Int`
* `Float`
* `Text`
* `Atom`
* lists of comparable values
* records of comparable values
* tuples of comparable values

Examples:

```zt
#prod == #prod
#prod != #dev
{ a = 1; } == { a = 1; }
(#ok, value = 1) == (#ok, value = 1)
```

Serializable values are a subset of these data values. Tuple values are comparable in `.zt`, but they do not have a direct `.zti` representation in v0.

Functions are not comparable.

Type values are not comparable in user code unless a future reflection API explicitly provides type identity operations.

Invalid:

```zt
(\x. x) == (\x. x)
Text == Text
```

---
