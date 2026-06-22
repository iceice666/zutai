use super::*;
use zutai_syntax::posit::{PositLiteral, PositSpec};

fn test_module(
    decls: Vec<SsaDecl>,
    entry: SsaFunc,
    entry_ty: DfTy,
    closure_exports: Vec<String>,
) -> SsaModule {
    let mut types = DfTypes::new();
    let entry_ty_id = types.alloc(entry_ty.clone());
    SsaModule {
        decls,
        entry,
        entry_ty,
        entry_ty_id,
        types,
        closure_exports,
    }
}

fn posit_module(spec: PositSpec, op: DfPositOp, entry_ty: DfTy) -> SsaModule {
    test_module(
        Vec::new(),
        SsaFunc {
            name: "__entry".to_string(),
            params: Vec::new(),
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: vec![SsaInstr {
                    dest: "result".to_string(),
                    op: SsaOp::Builtin {
                        op: DfBuiltinOp::Posit { op, spec },
                        lhs: SsaValue::Lit(DfLit::Posit(PositLiteral {
                            spec,
                            bits: 0x4000_0000,
                        })),
                        rhs: SsaValue::Lit(DfLit::Posit(PositLiteral {
                            spec,
                            bits: 0x4800_0000,
                        })),
                    },
                }],
                terminator: SsaTerminator::Return(SsaValue::Reg("result".to_string())),
            }],
        },
        entry_ty,
        Vec::new(),
    )
}

#[test]
fn coalesce_emits_runtime_helper_call() {
    let module = test_module(
        Vec::new(),
        SsaFunc {
            name: "__entry".to_string(),
            params: Vec::new(),
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: vec![SsaInstr {
                    dest: "result".to_string(),
                    op: SsaOp::Coalesce {
                        value: SsaValue::Lit(DfLit::Int(1)),
                        fallback: SsaValue::Lit(DfLit::Int(2)),
                    },
                }],
                terminator: SsaTerminator::Return(SsaValue::Reg("result".to_string())),
            }],
        },
        DfTy::Int,
        Vec::new(),
    );

    let llvm = emit_llvm(&module);
    assert!(llvm.contains("call i64 @zutai.coalesce"));
    assert!(!llvm.contains("icmp ne i64"), "{llvm}");
}

#[test]
fn record_update_emits_runtime_helper_call() {
    let module = test_module(
        Vec::new(),
        SsaFunc {
            name: "__entry".to_string(),
            params: Vec::new(),
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: vec![SsaInstr {
                    dest: "result".to_string(),
                    op: SsaOp::RecordUpdate {
                        base: SsaValue::Reg("base".to_string()),
                        updates: vec![(1, SsaValue::Lit(DfLit::Int(8080)))],
                    },
                }],
                terminator: SsaTerminator::Return(SsaValue::Reg("result".to_string())),
            }],
        },
        DfTy::Int,
        Vec::new(),
    );

    let llvm = emit_llvm(&module);
    assert!(llvm.contains("declare i64 @zutai.record_update"));
    assert!(llvm.contains("call i64 @zutai.record_update"));
    assert!(llvm.contains("call i64 @zutai.record_update(i64 %base, i64 1, i64 8080)"));
}

#[test]
fn posit32_builtin_emits_helper_call_with_truncation() {
    let spec = PositSpec { nbits: 32, es: 3 };
    let llvm = emit_llvm(&posit_module(spec, DfPositOp::Add, DfTy::Posit(spec)));
    assert!(llvm.contains("declare i32 @zutai.posit32e3.add(i32, i32)"));
    assert!(llvm.contains("trunc i64"));
    assert!(llvm.contains("call i32 @zutai.posit32e3.add"));
    assert!(llvm.contains("zext i32"));
}

#[test]
fn posit64_builtin_emits_helper_call_without_truncation() {
    let spec = PositSpec { nbits: 64, es: 5 };
    let llvm = emit_llvm(&posit_module(spec, DfPositOp::Add, DfTy::Posit(spec)));
    assert!(llvm.contains("declare i64 @zutai.posit64e5.add(i64, i64)"));
    assert!(llvm.contains("call i64 @zutai.posit64e5.add"));
    assert!(!llvm.contains("trunc i64"), "{llvm}");
}

#[test]
fn posit32_comparison_emits_bool_helper_and_zext() {
    let spec = PositSpec { nbits: 32, es: 3 };
    let llvm = emit_llvm(&posit_module(spec, DfPositOp::Lt, DfTy::Bool));
    assert!(llvm.contains("declare i1 @zutai.posit32e3.lt(i32, i32)"));
    assert!(llvm.contains("call i1 @zutai.posit32e3.lt"));
    assert!(llvm.contains("zext i1"));
}

#[test]
fn top_level_function_emits_static_closure() {
    let module = test_module(
        vec![SsaDecl::Func(SsaFunc {
            name: "inc".to_string(),
            params: vec!["__self".to_string(), "x".to_string()],
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: vec![SsaInstr {
                    dest: "r".to_string(),
                    op: SsaOp::Builtin {
                        op: DfBuiltinOp::Add,
                        lhs: SsaValue::Reg("x".to_string()),
                        rhs: SsaValue::Lit(DfLit::Int(1)),
                    },
                }],
                terminator: SsaTerminator::Return(SsaValue::Reg("r".to_string())),
            }],
        })],
        SsaFunc {
            name: "__entry".to_string(),
            params: Vec::new(),
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: Vec::new(),
                terminator: SsaTerminator::Return(SsaValue::Lit(DfLit::Int(0))),
            }],
        },
        DfTy::Int,
        vec!["inc".to_string()],
    );

    let llvm = emit_llvm(&module);
    assert!(
        llvm.contains("@zutai.closure.inc = internal constant { i64, ptr } { i64 7, ptr @inc }"),
        "{llvm}"
    );
    assert!(!llvm.contains("ptrtoint (ptr @"), "{llvm}");
}

#[test]
fn closure_apply_loads_code_and_passes_self() {
    let module = test_module(
        Vec::new(),
        SsaFunc {
            name: "__entry".to_string(),
            params: Vec::new(),
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: vec![SsaInstr {
                    dest: "result".to_string(),
                    op: SsaOp::ApplyClosure {
                        closure: SsaValue::GlobalClosure("inc".to_string()),
                        arg: SsaValue::Lit(DfLit::Int(41)),
                    },
                }],
                terminator: SsaTerminator::Return(SsaValue::Reg("result".to_string())),
            }],
        },
        DfTy::Int,
        Vec::new(),
    );

    let llvm = emit_llvm(&module);
    assert!(llvm.contains("getelementptr i64, ptr"), "{llvm}");
    assert!(llvm.contains("load i64, ptr"), "{llvm}");
    assert!(
        llvm.contains(" = ptrtoint ptr @zutai.closure.inc to i64"),
        "{llvm}"
    );
    // Code pointer is called indirectly with (self, arg).
    assert!(
        llvm.contains("call i64 %"),
        "indirect call expected: {llvm}"
    );
    // Legacy direct/raw call shapes are gone.
    assert!(!llvm.contains("call i64 @inc(i64 41)"), "{llvm}");
    assert!(!llvm.contains("to i64 (i64)*"), "{llvm}");
    assert!(!llvm.contains("ptrtoint (ptr @"), "{llvm}");
}

#[test]
fn capturing_lambda_allocates_heap_closure() {
    let module = test_module(
        Vec::new(),
        SsaFunc {
            name: "__entry".to_string(),
            params: Vec::new(),
            blocks: vec![SsaBlock {
                label: "entry".to_string(),
                instructions: vec![SsaInstr {
                    dest: "clos".to_string(),
                    op: SsaOp::MakeClosure {
                        code: "__lambda_0".to_string(),
                        captures: vec![SsaValue::Lit(DfLit::Int(10))],
                    },
                }],
                terminator: SsaTerminator::Return(SsaValue::Reg("clos".to_string())),
            }],
        },
        DfTy::Int,
        Vec::new(),
    );

    let llvm = emit_llvm(&module);
    // (2 + 1 capture) * 8 bytes = 24.
    assert!(llvm.contains("call i64 @zutai.alloc(i64 24)"), "{llvm}");
    // Header for one capture: (1 << 8) | 7 = 263.
    assert!(llvm.contains("store i64 263,"), "{llvm}");
    assert!(
        llvm.contains(" = ptrtoint ptr @__lambda_0 to i64"),
        "{llvm}"
    );
    // Capture stored at slot 2.
    assert!(llvm.contains(", i64 2\n"), "slot-2 gep expected: {llvm}");
    assert!(
        llvm.contains("store i64 10,"),
        "capture value stored: {llvm}"
    );
    assert!(!llvm.contains("ptrtoint (ptr @"), "{llvm}");
}
