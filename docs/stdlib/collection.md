# Collection constraints

`stdlib.collection` is an opt-in vocabulary for higher-kinded collection operations. It exports no runtime record fields; import it destructuring-style so its witnesses enter scope:

```zt
_ ::= import stdlib.collection;

Functor :: <F :: Type -> Type> @F {
  map :: <A, B> (A -> B) -> F A -> F B;
}

Foldable :: <F :: Type -> Type> @F {
  fold :: <A, B> (B -> A -> B) -> B -> F A -> B;
}

xs :: List Int = { 1; 2; 3; };
fold (\sum value. sum + value) 0 (map (\value. value * 2) xs)
```

The module provides coherent `Functor` and `Foldable` witnesses for `List`, `Optional`, and `Result E`. Importing it does not change the ambient `List` `map`/`fold` bindings; the constraint declarations in the importing module give those method spellings their witness-dispatched types. Existing specialized modules remain available as `stdlib.list`, `stdlib.optional`, and `stdlib.result`.

Support is full check/interpreter/native execution for all exported witnesses. `List` and `Optional` use imported bare-constructor witnesses; `Result E` uses an imported conditional first-order constructor witness, with evaluator and native package-boundary parity gates.
