## Conditionals

General mode uses `cond` for expression conditionals:

```zt
cond {
  condition => expr;
  _ => fallback;
}
```

Example:

```zt
port ::= cond {
  profile == #prod => 443;
  _ => 8080;
};
```

Each guard must have type `Bool`, arms are tried top-to-bottom, and the final
`_` branch is required. All branch bodies must type-check to a compatible type.

`cond` desugars to the core `if`/`else` conditional form used by later compiler
stages. The implementation still accepts `if condition then expr else expr` as
a legacy compatibility spelling.

---
