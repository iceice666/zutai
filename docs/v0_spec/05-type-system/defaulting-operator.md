## Defaulting operator

The defaulting operator is:

```zt
value ?? fallback
```

It unwraps exactly one `Optional` or `Maybe` layer:

```zt
match value {
  | #none        => fallback;
  | #some (x)    => x;
  | #absent      => fallback;
  | #present (x) => x;
}
```

Example:

```zt
raw.port ?? 8080
```

If `raw.port` is `#absent`, the result is `8080`.

If `raw.port` is `#present (p)`, the result is `p`.

If the value is `#present (#none)`, the result is `#none`; `??` removes only the outer `Maybe`.

---
