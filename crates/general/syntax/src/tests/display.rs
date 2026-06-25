use super::*;

// ============================================================================
// Display (display.rs) – exhaust every branch of write_decl / write_clause /
// write_expr / write_pattern / write_type_expr.
// ============================================================================

// ─── File header ─────────────────────────────────────────────────────────────

#[test]
fn display_file_starts_with_file_header() {
    let s = parse_str("42").to_string();
    assert!(s.starts_with("File\n"), "output must start with 'File\\n'");
}

// ─── Decl variants ───────────────────────────────────────────────────────────

#[test]
fn display_decl_inferred() {
    let s = parse_str("x ::= 42;\nx").to_string();
    assert!(s.contains("Inferred \"x\""), "inferred decl name");
    assert!(s.contains("Int(42)"), "inferred decl value");
}

#[test]
fn display_decl_typed() {
    let s = parse_str("x :: Int = 99;\nx").to_string();
    assert!(s.contains("Typed \"x\""), "typed decl name");
    assert!(s.contains("TyIdent(Int)"), "typed decl type annotation");
    assert!(s.contains("Int(99)"), "typed decl value");
}

#[test]
fn display_decl_type_alias_no_params() {
    let s = parse_str("MyInt :: type Int;\nMyInt").to_string();
    assert!(
        s.contains("TypeAlias \"MyInt\" <>"),
        "alias name and empty params"
    );
    assert!(s.contains("TyIdent(Int)"), "alias body");
}

#[test]
fn display_decl_type_alias_with_params() {
    let s = parse_str("Pair :: <A, B> type (A, B);\n1").to_string();
    assert!(
        s.contains("TypeAlias \"Pair\" <A, B>"),
        "alias with type params"
    );
}

#[test]
fn display_decl_function() {
    let s = parse_str("id :: Int -> Int\n  = x => x;\nid 1").to_string();
    assert!(s.contains("Function \"id\" <>"), "function decl name");
    assert!(s.contains("TyArrow"), "function signature");
    assert!(s.contains("Clause"), "function clause");
}

#[test]
fn display_decl_function_clause_with_guard() {
    let s = parse_str("pos :: Int -> Int\n  = x if x > 0 => x;\n  = _ => 0;\npos 3").to_string();
    assert!(s.contains("guard:"), "clause guard label");
    assert!(s.contains("Binary("), "guard binary expression");
}

#[test]
fn display_decl_nosig_fn() {
    let s = parse_str("f x = x;\nf 1").to_string();
    assert!(s.contains("NoSigFn \"f\""), "no-sig fn name");
    assert!(s.contains("pat: Ident(x)"), "no-sig fn pattern");
    assert!(s.contains("body: Ident(x)"), "no-sig fn body");
}

#[test]
fn display_decl_constraint() {
    let s = parse_str("Eq :: <A> @A { eq :: A -> A -> Bool; }\n1").to_string();
    assert!(s.contains("Constraint \"Eq\""), "constraint name");
}

#[test]
fn display_decl_witness() {
    let src = "Eq :: <A> @A { eq :: A -> A -> Bool; }\nEq @Int :: { eq = \\a b. true; }\n1";
    let s = parse_str(src).to_string();
    assert!(s.contains("Witness for \"Eq\""), "witness header");
}

// ─── Expr variants ───────────────────────────────────────────────────────────

#[test]
fn display_expr_true() {
    let s = parse_str("true").to_string();
    assert!(s.contains("final: true"), "true literal in final expr");
}

#[test]
fn display_expr_false() {
    let s = parse_str("false").to_string();
    assert!(s.contains("final: false"), "false literal in final expr");
}

#[test]
fn display_expr_integer() {
    let s = parse_str("123").to_string();
    assert!(s.contains("Int(123)"), "integer literal");
}

#[test]
fn display_expr_float() {
    let s = parse_str("3.14").to_string();
    assert!(s.contains("Float(3.14)"), "float literal");
}

#[test]
fn display_expr_posit() {
    let p32 = parse_str("1.5p32e3").to_string();
    assert!(p32.contains("Posit(Posit32e3,"), "p32e3 posit literal");

    let p64 = parse_str("1.5p64e5").to_string();
    assert!(p64.contains("Posit(Posit64e5,"), "p64e5 posit literal");
}

#[test]
fn display_expr_string() {
    let s = parse_str("\"hello\"").to_string();
    assert!(s.contains("Str(\"hello\")"), "string literal");
}

#[test]
fn display_expr_atom() {
    let s = parse_str("#foo").to_string();
    assert!(s.contains("Atom(#foo)"), "atom literal");
}

#[test]
fn display_expr_tagged_value() {
    let s = parse_str("#box { val = 1; }").to_string();
    assert!(s.contains("TaggedValue(#box)"), "tagged value tag");
    assert!(s.contains("Record"), "tagged value payload is record");
}

#[test]
fn display_expr_tagged_tuple_payload() {
    let s = parse_str("#pair (1, \"x\")").to_string();
    assert!(s.contains("TaggedValue(#pair)"), "tagged value tag");
    assert!(s.contains("Tuple"), "tagged value payload is tuple");
    assert!(s.contains("Int(1)"), "first tuple payload element");
}

#[test]
fn display_expr_ident() {
    let s = parse_str("x ::= 1;\nx").to_string();
    assert!(s.contains("Ident(x)"), "identifier expression");
}

#[test]
fn display_expr_record() {
    let s = parse_str("{ a = 1; b = 2; }").to_string();
    assert!(s.contains("Record"), "record expression");
    assert!(s.contains("a:"), "record field a");
    assert!(s.contains("b:"), "record field b");
}

#[test]
fn display_expr_record_update() {
    let s = parse_str("{ host = \"h\"; } with { host = \"n\"; }").to_string();
    assert!(s.contains("RecordUpdate"), "record update expression");
    assert!(s.contains("receiver:"), "record update receiver");
    assert!(s.contains("field host:"), "record update field");
}

#[test]
fn display_expr_tuple_positional() {
    let s = parse_str("(1, 2)").to_string();
    assert!(s.contains("Tuple"), "tuple expression");
    assert!(s.contains("Int(1)"), "first tuple element");
    assert!(s.contains("Int(2)"), "second tuple element");
}

#[test]
fn display_expr_tuple_named() {
    let s = parse_str("(x=1, y=2)").to_string();
    assert!(s.contains("Tuple"), "named tuple");
    assert!(s.contains("x="), "named tuple field x");
    assert!(s.contains("y="), "named tuple field y");
}

#[test]
fn display_expr_list() {
    let s = parse_str("{1; 2; 3;}").to_string();
    assert!(s.contains("List"), "list expression");
    assert!(s.contains("Int(1)"), "first list element");
}

#[test]
fn display_expr_block() {
    let s = parse_str("[ x := 1; x ]").to_string();
    assert!(s.contains("Block"), "block expression");
    assert!(s.contains("x:"), "block binding");
    assert!(s.contains("result:"), "block result");
}

#[test]
fn display_expr_lambda() {
    let s = parse_str(r"\x. x").to_string();
    assert!(s.contains("Lambda"), "lambda expression");
    assert!(s.contains("param:"), "lambda param");
    assert!(s.contains("body:"), "lambda body");
}

#[test]
fn display_expr_if() {
    let s = parse_str("if true then 1 else 2").to_string();
    assert!(s.contains("If"), "if expression");
    assert!(s.contains("cond:"), "if condition");
    assert!(s.contains("then:"), "if then branch");
    assert!(s.contains("else:"), "if else branch");
}

#[test]
fn display_expr_match() {
    let s = parse_str("match 1 { | 1 => true; | _ => false; }").to_string();
    assert!(s.contains("Match"), "match expression");
    assert!(s.contains("on:"), "match scrutinee");
    assert!(s.contains("Clause"), "match arm");
}

#[test]
fn display_decl_import_string() {
    let s = parse_str("cfg :: import \"data.zti\";\ncfg").to_string();
    assert!(s.contains("Import \"cfg\""), "string import decl");
    assert!(s.contains("source: \"data.zti\""), "string import source");
}

#[test]
fn display_decl_import_path() {
    let s = parse_str("cfg :: import foo.bar;\ncfg").to_string();
    assert!(s.contains("Import \"cfg\""), "path import decl");
    assert!(s.contains("source: foo.bar"), "path import source");
}

#[test]
fn display_expr_type_form() {
    let s = parse_str("type Int?").to_string();
    assert!(s.contains("TypeForm"), "type form expression");
    assert!(s.contains("TyOptional"), "type form contains optional type");
}

#[test]
fn display_expr_apply() {
    let s = parse_str("f 1").to_string();
    assert!(s.contains("Apply"), "application");
    assert!(s.contains("fn:"), "apply function");
    assert!(s.contains("arg:"), "apply argument");
}

#[test]
fn display_expr_access() {
    let s = parse_str("r ::= { a = 1; };\nr.a").to_string();
    assert!(s.contains("Access .a"), "field access");
}

#[test]
fn display_expr_opt_access() {
    let s = parse_str("r?.field").to_string();
    assert!(s.contains("OptAccess ?.field"), "optional field access");
}

#[test]
fn display_expr_binary() {
    let s = parse_str("1 + 2").to_string();
    assert!(s.contains("Binary("), "binary expression");
}

#[test]
fn display_expr_pipeline_forward() {
    let s = parse_str("1 |> f").to_string();
    assert!(s.contains("Pipeline(|>)"), "forward pipeline");
}

#[test]
fn display_expr_pipeline_backward() {
    let s = parse_str("f <| 1").to_string();
    assert!(s.contains("Pipeline(<|)"), "backward pipeline");
}

// ─── Pattern variants ─────────────────────────────────────────────────────────

#[test]
fn display_pattern_wildcard() {
    let s = parse_str("match 1 { | _ => 0; }").to_string();
    assert!(s.contains("pat: _"), "wildcard pattern");
}

#[test]
fn display_pattern_ident() {
    let s = parse_str("match 1 { | x => x; }").to_string();
    assert!(s.contains("pat: Ident(x)"), "ident pattern");
}

#[test]
fn display_pattern_true() {
    let s = parse_str("match true { | true => 1; | _ => 0; }").to_string();
    assert!(s.contains("pat: true"), "true pattern");
}

#[test]
fn display_pattern_false() {
    let s = parse_str("match false { | false => 0; | _ => 1; }").to_string();
    assert!(s.contains("pat: false"), "false pattern");
}

#[test]
fn display_pattern_integer() {
    let s = parse_str("match 42 { | 42 => true; | _ => false; }").to_string();
    assert!(s.contains("pat: Int(42)"), "integer pattern");
}

#[test]
fn display_pattern_float() {
    let s = parse_str("match 1.5 { | 1.5 => true; | _ => false; }").to_string();
    assert!(s.contains("pat: Float("), "float pattern");
}

#[test]
fn display_pattern_posit() {
    let s = parse_str("match 1p32 { | 1p32 => true; | _ => false; }").to_string();
    assert!(s.contains("pat: Posit("), "posit pattern");
}

#[test]
fn display_pattern_string() {
    let s = parse_str("match \"hi\" { | \"hi\" => 1; | _ => 0; }").to_string();
    assert!(s.contains("pat: Str("), "string pattern");
}

#[test]
fn display_pattern_atom() {
    let s = parse_str("match #ok { | #ok => 1; | _ => 0; }").to_string();
    assert!(s.contains("pat: Atom(#ok)"), "atom pattern");
}

#[test]
fn display_pattern_tagged_value() {
    let s = parse_str("match #box { v = 1; } { | #box { v = x; } => x; | _ => 0; }").to_string();
    assert!(s.contains("TaggedPat(#box)"), "tagged value pattern tag");
    assert!(s.contains("v="), "tagged pattern field");
}

#[test]
fn display_pattern_tagged_tuple_payload() {
    let s = parse_str("match #pair (1, 2) { | #pair (x, y) => x; | _ => 0; }").to_string();
    assert!(s.contains("TaggedPat(#pair)"), "tagged pattern tag");
    assert!(s.contains("0="), "first positional payload slot");
    assert!(s.contains("1="), "second positional payload slot");
}

#[test]
fn display_pattern_tuple_positional() {
    let s = parse_str("match (1, 2) { | (a, b) => a; }").to_string();
    assert!(s.contains("TuplePat"), "positional tuple pattern");
}

#[test]
fn display_pattern_tuple_named() {
    let s = parse_str("match (x=1, y=2) { | (x=a, y=b) => a; }").to_string();
    assert!(s.contains("TuplePat"), "named tuple pattern");
    assert!(s.contains("x="), "named tuple pattern field x");
    assert!(s.contains("y="), "named tuple pattern field y");
}

#[test]
fn display_pattern_record() {
    let s = parse_str("match { a = 1; } { | { a = x; } => x; }").to_string();
    assert!(s.contains("RecordPat"), "record pattern");
    assert!(s.contains("a="), "record pattern field");
}

// ─── TypeExpr variants ───────────────────────────────────────────────────────

#[test]
fn display_type_expr_ident() {
    let s = parse_str("x :: Int = 1;\nx").to_string();
    assert!(s.contains("TyIdent(Int)"), "type ident");
}

#[test]
fn display_type_expr_atom() {
    let s = parse_str("x :: #ok = #ok;\nx").to_string();
    assert!(s.contains("TyAtom(#ok)"), "type atom");
}

#[test]
fn display_type_expr_true() {
    let s = parse_str("x :: true = true;\nx").to_string();
    assert!(s.contains("TyTrue"), "type literal true");
}

#[test]
fn display_type_expr_false() {
    let s = parse_str("x :: false = false;\nx").to_string();
    assert!(s.contains("TyFalse"), "type literal false");
}

#[test]
fn display_type_expr_record_with_optional_field() {
    let s = parse_str("Point :: type { x : Int; y? : Text; };\n1").to_string();
    assert!(s.contains("TyRecord"), "type record");
    assert!(s.contains("x:"), "required field");
    assert!(s.contains("y?:"), "optional field");
}

#[test]
fn display_type_expr_union_with_and_without_payload() {
    let s = parse_str("Shape :: type {#circle; #rect: { w : Int; h : Int; };};\n1").to_string();
    assert!(s.contains("TyUnion"), "type union");
    assert!(s.contains("circle"), "bare union variant");
    assert!(s.contains("rect:"), "payload union variant");
}

#[test]
fn display_type_expr_tuple_positional() {
    let s = parse_str("f :: (Int, Text) -> Int\n  = _ => 0;\nf").to_string();
    assert!(s.contains("TyTuple"), "positional type tuple");
}

#[test]
fn display_type_expr_tuple_named() {
    let s = parse_str("T :: type (x : Int, y : Text);\n1").to_string();
    assert!(s.contains("TyTuple"), "named type tuple");
    assert!(s.contains("x:"), "named tuple type field x");
}

#[test]
fn display_type_expr_optional() {
    let s = parse_str("x :: Int? = #none;\nx").to_string();
    assert!(s.contains("TyOptional"), "optional type");
    assert!(s.contains("TyIdent(Int)"), "optional inner type");
}

#[test]
fn display_type_expr_arrow() {
    let s = parse_str("f :: Int -> Text\n  = _ => \"x\";\nf").to_string();
    assert!(s.contains("TyArrow"), "arrow type");
    assert!(s.contains("from:"), "arrow from");
    assert!(s.contains("to:"), "arrow to");
}

#[test]
fn display_type_expr_apply() {
    let s = parse_str("xs :: List Int = {1;};\nxs").to_string();
    assert!(s.contains("TyApply"), "type application");
}

#[test]
fn display_type_expr_access() {
    let s = parse_str("x :: Foo.Bar = x;\nx").to_string();
    assert!(s.contains("TyAccess .Bar"), "type field access");
}

#[test]
fn display_type_expr_expr_escape() {
    // A numeric literal in type position falls through to ExprEscape.
    let s = parse_str("x :: 1 = 1;\nx").to_string();
    assert!(s.contains("TyExprEscape"), "type expr escape");
    assert!(s.contains("Int(1)"), "escaped expression value");
}

#[test]
fn display_expr_perform_path() {
    let s = parse_str(r#"perform io.print "x""#).to_string();
    assert!(s.contains("Perform(io.print)"));
    assert!(s.contains(r#"Str("x")"#));
}

#[test]
fn display_expr_handle_resume() {
    let s = parse_str(r#"handle perform fail "bad" with { fail = \e. resume "ok"; }"#).to_string();
    assert!(s.contains("Handle"));
    assert!(s.contains("Perform(fail)"));
    assert!(s.contains("fail:"));
    assert!(s.contains("Resume"));
}

#[test]
fn display_expr_select() {
    let s = parse_str("select server { host; port; }").to_string();
    assert!(s.contains("Select"));
    assert!(s.contains("receiver: Ident(server)"));
    assert!(s.contains("field: host"));
    assert!(s.contains("field: port"));
}

#[test]
fn display_type_expr_select() {
    let s = parse_str("type select Server { host; port; }").to_string();
    assert!(s.contains("TySelect"));
    assert!(s.contains("TyIdent(Server)"));
    assert!(s.contains("field: host"));
    assert!(s.contains("field: port"));
}

#[test]
fn display_type_expr_effect_variants() {
    let s = parse_str("Eff :: type Unit ! { io.print : Text -> Unit, fail Error, tick };\n1")
        .to_string();
    assert!(s.contains("TyEffect"));
    assert!(s.contains("effect io.print: TyArrow"));
    assert!(s.contains("effect fail: TyIdent(Error)"));
    assert!(s.contains("effect tick"));
}

#[test]
fn display_type_expr_row_tails() {
    let record = parse_str("T :: type { host : Text; ...; };\n1").to_string();
    let union = parse_str("U :: type { #a; ...Rest; };\n1").to_string();
    assert!(record.contains("..."));
    assert!(union.contains("...Rest"));
}

#[test]
fn display_numeric_postfixes_on_exprs_and_patterns() {
    assert!(parse_str("1u8").to_string().contains("Int(1u8)"));
    assert!(parse_str("1f32").to_string().contains("Float(1f32)"));
    assert!(
        parse_str("match 1u8 { | 1u8 => true; | _ => false; }")
            .to_string()
            .contains("pat: Int(1u8)")
    );
    assert!(
        parse_str("match 1f32 { | 1f32 => true; | _ => false; }")
            .to_string()
            .contains("pat: Float(1f32)")
    );
}
