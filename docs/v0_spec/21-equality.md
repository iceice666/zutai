## 21. Equality

Structural equality is defined for serializable values:

* `none`
* `Bool`
* `Int`
* `Float`
* `Text`
* `Atom`
* lists of comparable values
* records of comparable values

Examples:

```zt
#prod == #prod
#prod != #dev
{ a = 1; } == { a = 1; }
```

Functions are not comparable.

Type values are not comparable in user code unless a future reflection API explicitly provides type identity operations.

Invalid:

```zt
(fn x => x) == (fn x => x)
Text == Text
```

---

