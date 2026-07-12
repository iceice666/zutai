## Equality

Structural equality is defined for first-order data values:

* `Bool`
* `Int`
* `Float`
* posit scalar types (`Posit32`, `Posit64`, `Posit32eN`, `Posit64eN`)
* `Text`
* `Atom`
* lists of comparable values
* records of comparable values
* tuples of comparable values
* tagged union values whose payloads are comparable

Examples:

```zt
#prod == #prod
#prod != #dev
{ a = 1; } == { a = 1; }
#ok { value = 1; } == #ok { value = 1; }
```

Serializable values are a subset of these data values. Tagged union values and tuple values are comparable in `.zt`, but they do not have a direct `.zti` representation in Zutai.

Functions are not comparable.

Type values are not comparable in user code unless a future reflection API explicitly provides type identity operations.

Invalid:

```zt
(\x. x) == (\x. x)
Text == Text
```

---
