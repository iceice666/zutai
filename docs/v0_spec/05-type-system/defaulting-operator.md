## Defaulting operator

The defaulting operator is:

```zt
value ?? fallback
```

It is shorthand for:

```zt
match value {
  #none => fallback;
  (#some, value = x) => x;
}
```

Example:

```zt
raw.port ?? 8080
```

If `raw.port` is `#none`, the result is `8080`.

If `raw.port` is `(#some, value = p)`, the result is `p`.

---
