use crate::*;

/// Build an SsaModule from a source string. Panics on any diagnostic.
fn ssa_of(src: &str) -> SsaModule {
    let parsed = zutai_syntax::parse(src);
    assert!(
        !parsed.has_errors(),
        "parse errors: {:?}",
        parsed.diagnostics()
    );
    let hir = zutai_hir::lower_file(parsed.ast().expect("parse AST"));
    assert!(
        hir.diagnostics.is_empty(),
        "HIR errors: {:?}",
        hir.diagnostics
    );
    let thir = zutai_thir::lower_hir(&hir.file);
    assert!(
        thir.diagnostics.is_empty(),
        "THIR errors: {:?}",
        thir.diagnostics
    );
    let tlc = zutai_tlc::lower_thir(thir.file.as_ref().expect("THIR file should be complete"));
    let dc = zutai_dataflow::lower_tlc(&tlc, &hir.file.bindings);
    let anf = zutai_anf::lower_dc(&dc);
    lower_anf(&anf)
}

/// Collect all instruction op names in a function.
fn op_names(func: &SsaFunc) -> Vec<String> {
    func.blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .map(|i| match &i.op {
            SsaOp::ApplyClosure { .. } => "ApplyClosure".to_string(),
            SsaOp::HostPrint { .. } => "HostPrint".to_string(),
            SsaOp::MakeClosure { .. } => "MakeClosure".to_string(),
            SsaOp::LoadCapture { .. } => "LoadCapture".to_string(),
            SsaOp::CallGlobal { .. } => "CallGlobal".to_string(),
            SsaOp::TyApp { .. } => "TyApp".to_string(),
            SsaOp::Record { .. } => "Record".to_string(),
            SsaOp::RecordUpdate { .. } => "RecordUpdate".to_string(),
            SsaOp::Tuple { .. } => "Tuple".to_string(),
            SsaOp::List { .. } => "List".to_string(),
            SsaOp::Select { .. } => "Select".to_string(),
            SsaOp::Variant { .. } => "Variant".to_string(),
            SsaOp::VariantValue { .. } => "VariantValue".to_string(),
            SsaOp::Builtin { .. } => "Builtin".to_string(),
            SsaOp::Coalesce { .. } => "Coalesce".to_string(),
            SsaOp::Error => "Error".to_string(),
            SsaOp::Alias { .. } => "Alias".to_string(),
            SsaOp::Phi { .. } => "Phi".to_string(),
            SsaOp::MatchDiscriminant { .. } => "MatchDiscriminant".to_string(),
        })
        .collect()
}

/// Collect all terminator kinds in a function.
fn terminator_kinds(func: &SsaFunc) -> Vec<String> {
    func.blocks
        .iter()
        .map(|b| match &b.terminator {
            SsaTerminator::Return(_) => "Return".to_string(),
            SsaTerminator::Jump(_) => "Jump".to_string(),
            SsaTerminator::Branch { .. } => "Branch".to_string(),
        })
        .collect()
}

/// Collect op names across all functions in the module (entry + decls).
fn all_op_names(module: &SsaModule) -> Vec<String> {
    let mut ops = op_names(&module.entry);
    for decl in &module.decls {
        match decl {
            SsaDecl::Func(f) => ops.extend(op_names(f)),
            SsaDecl::RecGroup(funcs) => {
                for f in funcs {
                    ops.extend(op_names(f));
                }
            }
        }
    }
    ops
}

/// Collect all functions (entry + all decls).
fn all_funcs(module: &SsaModule) -> Vec<&SsaFunc> {
    let mut funcs = vec![&module.entry];
    for decl in &module.decls {
        match decl {
            SsaDecl::Func(f) => funcs.push(f),
            SsaDecl::RecGroup(funcs_) => {
                for f in funcs_ {
                    funcs.push(f);
                }
            }
        }
    }
    funcs
}

/// Collect all terminators across all functions.
fn all_terminator_kinds(module: &SsaModule) -> Vec<String> {
    let mut terms = Vec::new();
    for f in all_funcs(module) {
        terms.extend(terminator_kinds(f));
    }
    terms
}

/// Collect all instructions across all functions.
fn all_instructions(module: &SsaModule) -> Vec<&SsaInstr> {
    let mut instrs = Vec::new();
    for f in all_funcs(module) {
        for b in &f.blocks {
            instrs.extend(b.instructions.iter());
        }
    }
    instrs
}

// ── Entry function exists ──────────────────────────────────────────────────────

#[test]
fn entry_function_exists() {
    let m = ssa_of("42");
    assert_eq!(m.entry.name, "__entry");
    assert!(!m.entry.blocks.is_empty());
    assert_eq!(m.entry.params.len(), 0);
}

// ── Integer literal ────────────────────────────────────────────────────────────

#[test]
fn int_literal_produces_return() {
    let m = ssa_of("42");
    let entry = &m.entry;
    let last_block = entry.blocks.last().expect("entry has blocks");
    match &last_block.terminator {
        SsaTerminator::Return(val) => match val {
            SsaValue::Lit(DfLit::Int(42)) => {}
            other => panic!("expected Return(Lit(Int(42))), got {:?}", other),
        },
        other => panic!("expected Return terminator, got {:?}", other),
    }
}

// ── Block-local let binding ────────────────────────────────────────────────────

#[test]
fn block_let_binding_produces_instructions() {
    let m = ssa_of("{ n := 42; n + n }");
    let ops = op_names(&m.entry);
    assert!(
        ops.contains(&"Builtin".to_string()),
        "should have a Builtin(Add) instruction: {:?}",
        ops
    );
}

// ── Function call ──────────────────────────────────────────────────────────────

#[test]
fn function_call_produces_apply_closure_op() {
    let m = ssa_of("id x = x\nid 42");
    let ops = all_op_names(&m);
    assert!(
        ops.contains(&"ApplyClosure".to_string()),
        "should have an ApplyClosure op: {:?}",
        ops
    );
}

// ── Lambda creates lifted function ──────────────────────────────────────────────

#[test]
fn lambda_creates_separate_function() {
    let m = ssa_of("inc x = x + 1\ninc 3");
    let inc = m.decls.iter().find_map(|d| match d {
        SsaDecl::Func(f) if f.name == "inc" => Some(f),
        _ => None,
    });
    assert!(inc.is_some(), "should have an 'inc' function");
}

// ── Top-level function becomes a static closure export ─────────────────────────

#[test]
fn top_level_function_exports_closure_value() {
    let m = ssa_of("inc :: Int -> Int\n  = x => x + 1;\ninc 41");
    assert_eq!(
        m.closure_exports,
        vec!["inc".to_string()],
        "inc should be exported as a static closure: {:?}",
        m.closure_exports
    );
    let inc = m
        .decls
        .iter()
        .find_map(|d| match d {
            SsaDecl::Func(f) if f.name == "inc" => Some(f),
            _ => None,
        })
        .expect("inc function decl");
    assert_eq!(
        inc.params.len(),
        2,
        "closure-code fn takes (self, arg): {:?}",
        inc.params
    );
    assert_eq!(inc.params[0], "__self", "first param is the closure self");
}

// ── Capturing lambda allocates a closure and loads captures ────────────────────

#[test]
fn capturing_lambda_uses_make_closure_and_load_capture() {
    // `adder n x = x + n` curries to `\n. \x. x + n`; the inner lambda captures
    // the outer parameter `n` (a genuine local that survives constant folding).
    let m = ssa_of("adder n x = x + n\nadder 10 5");
    let ops = all_op_names(&m);
    assert!(
        ops.contains(&"MakeClosure".to_string()),
        "should allocate a closure for the capturing lambda: {ops:?}"
    );
    assert!(
        ops.contains(&"LoadCapture".to_string()),
        "should load the captured variable: {ops:?}"
    );
    // The legacy `__fn` closure-record hack must be gone; this program has no
    // user records, so any Record op would be that removed representation.
    let has_record = all_instructions(&m)
        .iter()
        .any(|i| matches!(i.op, SsaOp::Record { .. }));
    assert!(!has_record, "no closure Record op should remain");
}

// ── Top-level let declaration ──────────────────────────────────────────────────

#[test]
fn top_level_let_produces_func_decl() {
    let m = ssa_of("x ::= 42\nx");
    let has_x_func = m
        .decls
        .iter()
        .any(|d| matches!(d, SsaDecl::Func(f) if f.name == "x"));
    assert!(
        has_x_func,
        "should have a Func decl named 'x': {:?}",
        m.decls
    );
}

// ── Recursive let produces RecGroup ────────────────────────────────────────────

#[test]
fn recursive_let_produces_rec_group() {
    let m = ssa_of("factorial n = if n < 1 then 1 else n * factorial (n - 1)\nfactorial 5");
    let has_rec_group = m.decls.iter().any(|d| matches!(d, SsaDecl::RecGroup(_)));
    assert!(
        has_rec_group,
        "should have a RecGroup for recursive decl: {:?}",
        m.decls
    );
}

// ── Record literal ─────────────────────────────────────────────────────────────

#[test]
fn record_literal_produces_record_op() {
    let m = ssa_of("{ x = 1; y = 2; }");
    let ops = op_names(&m.entry);
    assert!(
        ops.contains(&"Record".to_string()),
        "should have a Record op: {:?}",
        ops
    );
}

#[test]
fn record_update_produces_record_update_op() {
    let m = ssa_of("r ::= { x = 1; y = 2; }\nr with { x = 3; }");
    let ops = all_op_names(&m);
    assert!(
        ops.contains(&"RecordUpdate".to_string()),
        "should have a RecordUpdate op: {:?}",
        ops
    );
}

// ── Tuple literal ──────────────────────────────────────────────────────────────

#[test]
fn tuple_literal_produces_tuple_op() {
    let m = ssa_of("(1, 2, 3)");
    let ops = op_names(&m.entry);
    assert!(
        ops.contains(&"Tuple".to_string()),
        "should have a Tuple op: {:?}",
        ops
    );
}

// ── List literal ──────────────────────────────────────────────────────────────

#[test]
fn list_literal_produces_list_op() {
    let m = ssa_of("[1; 2; 3;]");
    let ops = op_names(&m.entry);
    assert!(
        ops.contains(&"List".to_string()),
        "should have a List op: {:?}",
        ops
    );
}

// ── Field selection ────────────────────────────────────────────────────────────

#[test]
fn field_selection_produces_select_op() {
    let m = ssa_of("r ::= { x = 1; }\nr.x");
    let ops = all_op_names(&m);
    assert!(
        ops.contains(&"Select".to_string()),
        "should have a Select op: {:?}",
        ops
    );
}

// ── Binary operation ───────────────────────────────────────────────────────────

#[test]
fn binary_op_produces_builtin_op() {
    let m = ssa_of("1 + 2");
    let ops = op_names(&m.entry);
    assert!(
        ops.contains(&"Builtin".to_string()),
        "should have a Builtin op: {:?}",
        ops
    );
}

// ── Variant construction ──────────────────────────────────────────────────────

#[test]
fn variant_produces_variant_op() {
    let src = "\
Status :: type {
  #ok: { code : Int; };
  #err: { msg : Text; };
}
s :: Status = #ok { code = 200; }
s";
    let m = ssa_of(src);
    let ops = all_op_names(&m);
    assert!(
        ops.contains(&"Variant".to_string()),
        "should have a Variant op: {:?}",
        ops
    );
}

// ── If desugars to match (produces branches and Phi) ───────────────────────────

#[test]
fn if_desugars_to_branching_match_in_ssa() {
    let m = ssa_of("f x = if x then 1 else 2\nf true");
    let all_ops = all_op_names(&m);
    let terms = all_terminator_kinds(&m);
    assert!(
        terms.contains(&"Branch".to_string()),
        "if should produce Branch terminators: {:?}",
        terms
    );
    assert!(
        all_ops.contains(&"Phi".to_string()),
        "if should produce Phi: {:?}",
        all_ops
    );
}

// ── Match expression produces multiple blocks ─────────────────────────────────

#[test]
fn match_expression_produces_multiple_blocks() {
    let m = ssa_of("f x = match x { | 1 => true; | _ => false; }\nf 1");
    let funcs = all_funcs(&m);
    let match_func = funcs.iter().find(|f| f.blocks.len() >= 3);
    assert!(
        match_func.is_some(),
        "should have a function with ≥3 blocks (arms + join), got: {:?}",
        funcs
            .iter()
            .map(|f| (&f.name, f.blocks.len()))
            .collect::<Vec<_>>()
    );
}

// ── Entry function return terminator ───────────────────────────────────────────

#[test]
fn entry_blocks_end_with_return() {
    let m = ssa_of("42");
    let last = m.entry.blocks.last().expect("has blocks");
    assert!(
        matches!(last.terminator, SsaTerminator::Return(_)),
        "last block should return, got {:?}",
        last.terminator
    );
}

// ── Jump terminators exist in match/if ─────────────────────────────────────────

#[test]
fn match_creates_jump_terminators() {
    let m = ssa_of("f x = if x then 1 else 2\nf true");
    let terms = all_terminator_kinds(&m);
    assert!(
        terms.contains(&"Jump".to_string()),
        "should have Jump terminators: {:?}",
        terms
    );
}

// ── Module is well-formed ─────────────────────────────────────────────────────

#[test]
fn module_is_well_formed() {
    let m = ssa_of("id x = x\nid 1");
    assert!(!m.entry.blocks.is_empty());
    let last = m.entry.blocks.last().unwrap();
    assert!(matches!(last.terminator, SsaTerminator::Return(_)));
}

// ── Phi instruction exists in if/match ────────────────────────────────────────

#[test]
fn phi_instruction_exists_in_if() {
    // "if" desugars to a match, which creates phi nodes at join points.
    let m = ssa_of("f x = if x then 1 else 2\nf true");
    let has_phi = all_instructions(&m)
        .iter()
        .any(|i| matches!(i.op, SsaOp::Phi { .. }));
    assert!(has_phi, "if should produce at least one Phi instruction");
}

// ── Coalesce operator ─────────────────────────────────────────────────────────

#[test]
fn coalesce_produces_coalesce_op() {
    let src = "\
RawServer :: type {
  port? : Int;
}
server :: RawServer = {}
server.port ?? 8080";
    let m = ssa_of(src);
    let ops = all_op_names(&m);
    assert!(
        ops.contains(&"Coalesce".to_string()),
        "should have a Coalesce op: {:?}",
        ops
    );
}
