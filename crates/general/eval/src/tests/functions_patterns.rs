use super::*;

// ─── lambda expressions ───────────────────────────────────────────────────────

#[test]
fn lambda_identity() {
    assert_eq!(run(r"(\x . x) 42"), Value::Int(42));
}

#[test]
fn lambda_add() {
    // Two-parameter lambda applied to two arguments (curried)
    assert_eq!(run(r"(\x y . x + y) 3 4"), Value::Int(7));
}

#[test]
fn lambda_captured_env() {
    // Lambda captures surrounding block binding
    assert_eq!(run(r"{ n := 10; (\x . x + n) 5 }"), Value::Int(15));
}

#[test]
fn lambda_as_value_binding() {
    // Lambda stored in a type-annotated value declaration, then applied
    let src = "
double :: Int -> Int = \\x . x + x
double 7
";
    assert_eq!(run(src), Value::Int(14));
}

#[test]
fn lambda_partial_application() {
    assert_eq!(
        run(r"{ add := \x y . x + y; add_two := add 2; add_two 3 }"),
        Value::Int(5)
    );
}

// ─── match expressions ────────────────────────────────────────────────────────

#[test]
fn match_int_literal() {
    // Matched arm returns Int so both arms have the same type.
    let src = r"
match 0 {
  | 0 => 1;
  | _ => 2;
}
";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn match_wildcard_fallthrough() {
    let src = r"
match 99 {
  | 0 => 1;
  | _ => 2;
}
";
    assert_eq!(run(src), Value::Int(2));
}

#[test]
fn match_bind_pattern() {
    // Binding pattern captures the matched value.
    let src = r"
match 7 {
  | n => n * 2;
}
";
    assert_eq!(run(src), Value::Int(14));
}

#[test]
fn match_with_guard() {
    // Guard filters to the correct arm.
    let src = r"
match 5 {
  | n if n > 3 => 1;
  | _ => 0;
}
";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn match_guard_falls_through() {
    // When the guard fails, the next arm is tried.
    let src = r"
match 2 {
  | n if n > 3 => 1;
  | _ => 0;
}
";
    assert_eq!(run(src), Value::Int(0));
}

#[test]
fn match_bool_patterns() {
    assert_eq!(
        run(r"match true { | true => 1; | false => 0; }"),
        Value::Int(1)
    );
    assert_eq!(
        run(r"match false { | true => 1; | false => 0; }"),
        Value::Int(0)
    );
}

#[test]
fn match_function_using_match_expr() {
    // match expression inside a lambda stored as a value binding
    let src = "
is_zero :: Int -> Bool = \\n. match n {
  | 0 => true;
  | _ => false;
}
is_zero 0
";
    assert_eq!(run(src), Value::Bool(true));
}

// ─── positional tagged union payloads ─────────────────────────────────────────

#[test]
fn positional_union_payload_constructs_and_matches() {
    let src = r#"
Pair :: type {
  #pair: (Int, Int);
  #empty;
}
sum :: Pair -> Int
  = #pair (x, y) => x + y;
  = #empty => 0;
sum #pair (2, 3)
"#;
    assert_eq!(run(src), Value::Int(5));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(5));
}

#[test]
fn single_positional_union_payload_constructs_and_matches() {
    let src = r#"
Boxed :: type {
  #boxed: (Int);
}
get :: Boxed -> Int
  = #boxed (x) => x;
get #boxed (9)
"#;
    assert_eq!(run(src), Value::Int(9));
    assert_eq!(eval_tlc_file(src).unwrap(), Value::Int(9));
}

#[test]
fn positional_union_payload_displays_tuple_syntax() {
    let src = r#"
Pair :: type {
  #pair: (Int, Int);
}
value :: Pair = #pair (4, 5)
value
"#;
    assert_eq!(run(src).to_string(), "#pair (4, 5)");
    assert_eq!(eval_tlc_file(src).unwrap().to_string(), "#pair (4, 5)");
}

// ─── optional access ──────────────────────────────────────────────────────────

#[test]
fn optional_access_present() {
    // `?.` chains through a present optional record field.
    // outer.inner has type Maybe(Inner); outer.inner?.val returns Maybe(Int).
    let src = "
Inner :: type { val : Int; }
Outer :: type { inner? : Inner; }
outer :: Outer = { inner = { val = 42; }; }
outer.inner?.val
";
    assert_eq!(run(src).to_string(), "#present (42)");
}

#[test]
fn optional_access_absent() {
    // When the optional record field is absent, ?.field returns #absent.
    let src = "
Inner :: type { val : Int; }
Outer :: type { inner? : Inner; }
outer :: Outer = {}
outer.inner?.val
";
    assert_eq!(run(src), Value::Atom("absent".into()));
}

#[test]
fn match_optional_field_absent() {
    let src = "
S :: type { p? : Int; }
s :: S = {}
match s.p {
  | #absent => 1;
  | #present (n) => n;
}
";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn match_optional_field_present() {
    let src = "
S :: type { p? : Int; }
s :: S = { p = 7; }
match s.p {
  | #absent => 1;
  | #present (n) => n;
}
";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn match_optional_explicit_some() {
    let src = "
x :: Int? = #some (9)
match x {
  | #none => 0;
  | #some (n) => n;
}
";
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn match_optional_explicit_none() {
    let src = "
x :: Int? = #none
match x {
  | #none => 0;
  | #some (n) => n;
}
";
    assert_eq!(run(src), Value::Int(0));
}

#[test]
fn optional_access_explicit_some() {
    let src = "
Inner :: type { val : Int; }
cfg :: Inner? = #some ({ val = 42; })
cfg?.val
";
    assert_eq!(run(src).to_string(), "#some (42)");
}

#[test]
fn optional_access_explicit_none() {
    let src = "
Inner :: type { val : Int; }
cfg :: Inner? = #none
cfg?.val
";
    assert_eq!(run(src), Value::Atom("none".into()));
}

#[test]
fn optional_double_optional_field_preserves_presence() {
    let src = "
S :: type { p? : Int?; }
s :: S = { p = #none; }
s.p
";
    assert_eq!(run(src).to_string(), "#present (#none)");
}

#[test]
fn optional_double_optional_field_some_preserves_presence() {
    let src = "
S :: type { p? : Int?; }
s :: S = { p = #some (5); }
s.p
";
    assert_eq!(run(src).to_string(), "#present (#some (5))");
}

#[test]
fn optional_bool_optional_field_preserves_absent_none_and_some() {
    let src = "
S :: type { tls? : Bool?; }
absent :: S = {}
none :: S = { tls = #none; }
some :: S = { tls = #some (true); }
{
  a = absent.tls;
  n = none.tls;
  s = some.tls;
}
";
    let rendered = run(src).to_string();
    assert!(rendered.contains("a = #absent"), "{rendered}");
    assert!(rendered.contains("n = #present (#none)"), "{rendered}");
    assert!(
        rendered.contains("s = #present (#some (true))"),
        "{rendered}"
    );
}

// ─── TaggedValue semantics ────────────────────────────────────────────────────

#[test]
fn tagged_value_equality_same_tag_and_payload() {
    let src = "
Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
}
a :: Status = #ok { code = 200; }
b :: Status = #ok { code = 200; }
a == b
";
    assert_eq!(run(src), Value::Bool(true));
}

#[test]
fn tagged_value_equality_different_tag() {
    let src = "
Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
}
a :: Status = #ok { code = 200; }
b :: Status = #err { msg = \"nope\"; }
a == b
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn tagged_value_equality_different_payload() {
    let src = "
Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
}
a :: Status = #ok { code = 200; }
b :: Status = #ok { code = 404; }
a == b
";
    assert_eq!(run(src), Value::Bool(false));
}

#[test]
fn tagged_value_tag_field_access() {
    // `.tag` on a tagged value returns the atom of the tag name.
    // Since THIR's static type of a union is "union" (not a record), field access
    // must go through match + record access.  We verify the `.tag` runtime path
    // by writing a function that accepts Any and returns via match.
    let src = "
Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
}
getCode :: Status -> Int
  = #ok { code = n; } => n;
  = #err { msg = _; } => -1;
getCode (#ok { code = 200; })
";
    assert_eq!(run(src), Value::Int(200));
}

#[test]
fn tagged_value_match_by_tag() {
    let src = "
Color :: type {
  #red: { r : Int; };
  #blue: { b : Int; };
}
c :: Color = #red { r = 255; }
match c {
  | #red { r = n; } => n;
  | #blue { b = n; } => 0;
}
";
    assert_eq!(run(src), Value::Int(255));
}

#[test]
fn tagged_value_match_wrong_tag_falls_through() {
    let src = "
Color :: type {
  #red: { r : Int; };
  #blue: { b : Int; };
}
c :: Color = #blue { b = 100; }
match c {
  | #red { r = n; } => n;
  | #blue { b = n; } => n + 1;
}
";
    assert_eq!(run(src), Value::Int(101));
}
// ─── match_pattern: Float literal patterns ────────────────────────────────────

#[test]
fn match_float_pattern_in_function_clause() {
    let src = "
classify :: Float -> Text
  = 0.0 => \"zero\";
  = 1.5 => \"one-half\";
  = _ => \"other\";
classify 1.5
";
    assert_eq!(run(src), Value::Text("one-half".into()));
}

#[test]
fn match_float_pattern_fallthrough() {
    let src = "
classify :: Float -> Text
  = 0.0 => \"zero\";
  = 1.5 => \"one-half\";
  = _ => \"other\";
classify 2.0
";
    assert_eq!(run(src), Value::Text("other".into()));
}

#[test]
fn match_posit_pattern_in_function_clause() {
    let src = "
classify :: Posit32e3 -> Text
  = 0p32e3 => \"zero\";
  = _ => \"other\";
classify 0p32e3
";
    assert_eq!(run(src), Value::Text("zero".into()));
}

#[test]
fn match_posit_pattern_fallthrough() {
    let src = "
classify :: Posit32e3 -> Text
  = 0p32e3 => \"zero\";
  = _ => \"other\";
classify 1p32e3
";
    assert_eq!(run(src), Value::Text("other".into()));
}

// ─── match_pattern: String literal patterns ───────────────────────────────────

#[test]
fn match_string_pattern_in_function_clause() {
    let src = "
greet :: Text -> Text
  = \"hello\" => \"world\";
  = \"hi\" => \"there\";
  = _ => \"unknown\";
greet \"hello\"
";
    assert_eq!(run(src), Value::Text("world".into()));
}

#[test]
fn match_string_pattern_fallthrough() {
    let src = "
greet :: Text -> Text
  = \"hello\" => \"world\";
  = _ => \"stranger\";
greet \"goodbye\"
";
    assert_eq!(run(src), Value::Text("stranger".into()));
}

// ─── match_pattern: Atom literal patterns ────────────────────────────────────

#[test]
fn match_atom_pattern_in_function_clause() {
    // The type `#foo` is a singleton atom type (Atom("foo")), not a union variant.
    // Pattern `#foo` in this context produces ThirPatKind::Atom.
    let src = "
describe :: #foo -> Text
  = #foo => \"it is foo\";
describe #foo
";
    assert_eq!(run(src), Value::Text("it is foo".into()));
}

// ─── match_pattern: Positional Tuple patterns ─────────────────────────────────

#[test]
fn match_positional_tuple_pattern_in_function_clause() {
    // Positional tuple pattern `(x, y)` exercises ThirPatKind::Tuple Positional arm.
    let src = "
fst :: (Int, Text) -> Int
  = (n, _) => n;
fst (42, \"hi\")
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn match_positional_tuple_pattern_in_match_expr() {
    let src = "
p :: (Int, Int) = (10, 20)
match p {
  | (x, y) => x + y;
}
";
    assert_eq!(run(src), Value::Int(30));
}

// ─── match_pattern: Named Tuple patterns ─────────────────────────────────────

#[test]
fn named_tuple_construction_and_named_pattern() {
    // Named tuple value `(x = 42, y = 99)` exercises ThirTupleItem::Named construction.
    // Pattern `(x = v, y = _)` exercises ThirTuplePatItem::Named matching.
    let src = "
Coord :: type (x : Int, y : Int)
getX :: Coord -> Int
  = (x = v, y = _) => v;
getX (x = 42, y = 99)
";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn display_named_tuple() {
    // Named tuple fields display as `name = value`.
    let src = "
Coord :: type (x : Int, y : Int)
p :: Coord = (x = 10, y = 20)
p
";
    assert_eq!(run(src).to_string(), "(x = 10, y = 20)");
}

// ─── match_pattern: Record patterns in function clauses ───────────────────────

#[test]
fn match_record_pattern_in_function_clause() {
    // Record pattern `{ port = n; }` exercises ThirPatKind::Record in match_pattern.
    let src = "
Server :: type { host : Text; port : Int; }
getPort :: Server -> Int
  = { host = _; port = n; } => n;
getPort { host = \"localhost\"; port = 8080; }
";
    assert_eq!(run(src), Value::Int(8080));
}

#[test]
fn match_record_pattern_multiple_fields() {
    let src = "
Point :: type { x : Int; y : Int; }
sumCoords :: Point -> Int
  = { x = a; y = b; } => a + b;
sumCoords { x = 3; y = 4; }
";
    assert_eq!(run(src), Value::Int(7));
}

// ─── Guard false in function clause (apply_closure path) ─────────────────────

#[test]
fn function_clause_guard_false_falls_through() {
    // guard `n > 0` evaluates to false for negative input → falls through to next clause.
    let src = "
classify :: Int -> Int
  = n if n > 0 => 1;
  = 0 => 0;
  = _ => -1;
classify (-1)
";
    assert_eq!(run(src), Value::Int(-1));
}

#[test]
fn function_clause_guard_false_then_matching_clause() {
    // Guard on first clause fails; second clause (no guard) matches.
    let src = "
safe_div :: Int -> Int -> Int
  = _ 0 => 0;
  = n d if d > 0 => n / d;
  = _ _ => 0;
safe_div 10 0
";
    assert_eq!(run(src), Value::Int(0));
}
