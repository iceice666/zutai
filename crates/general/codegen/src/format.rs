use crate::instr::alloc_tmp;
use zutai_ssa::*;

/// Sanitise a Zutai identifier into a valid LLVM IR name.
pub(crate) fn mangle(name: &str) -> String {
    let sanitized = name
        .replace(['-', '.', '='], "_")
        .replace('?', "_Q")
        .replace('!', "_B")
        .replace('@', "_at_");
    // `main` is the C entry symbol `emit_main` emits verbatim (`define i32
    // @main`); a user binding named `main` would redefine it. `$` cannot occur
    // in a source identifier (UAX #31 — only synthesized names like witness
    // globals use it), so a `$`-marked rename is collision-free with both source
    // names and the `$dep…` witness scheme.
    if sanitized == "main" {
        return "main$user".to_string();
    }
    sanitized
}

/// D-0003 closure object tag (matches `TAG_CLOSURE` in `zutai-rt`).
pub(crate) const CLOSURE_TAG: u64 = 7;

/// Pack a closure header word: low byte = tag, next bits = capture count.
pub(crate) fn closure_header(ncaps: usize) -> u64 {
    ((ncaps as u64) << 8) | CLOSURE_TAG
}

pub(crate) const TAG_TEXT: u64 = 6;

pub(crate) const DESC_INT: i64 = 0;

pub(crate) const DESC_BOOL: i64 = 1;

pub(crate) const DESC_FLOAT: i64 = 2;

pub(crate) const DESC_TEXT: i64 = 3;

pub(crate) const DESC_ATOM: i64 = 4;

pub(crate) const DESC_RECORD: i64 = 5;

pub(crate) const DESC_TUPLE: i64 = 6;

pub(crate) const DESC_LIST: i64 = 7;

pub(crate) const DESC_OPTIONAL: i64 = 8;

pub(crate) const DESC_MAYBE: i64 = 9;

pub(crate) const DESC_VARIANT: i64 = 10;

pub(crate) const DESC_POSIT: i64 = 11;

pub(crate) fn object_header(tag: u64, count: usize) -> u64 {
    ((count as u64) << 8) | tag
}

/// Global symbol name of the static closure object for a top-level function.
pub(crate) fn closure_global_name(name: &str) -> String {
    format!("zutai.closure.{}", mangle(name))
}

pub(crate) enum StaticWord {
    I64(String),
    Ptr(String),
}

pub(crate) fn i64_word(value: impl std::fmt::Display) -> StaticWord {
    StaticWord::I64(value.to_string())
}

pub(crate) fn ptr_word(symbol: impl Into<String>) -> StaticWord {
    StaticWord::Ptr(symbol.into())
}

pub(crate) fn emit_static_words(out: &mut String, name: &str, linkage: &str, words: &[StaticWord]) {
    out.push('@');
    out.push_str(name);
    out.push_str(" = ");
    out.push_str(linkage);
    out.push_str(" constant { ");
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        match word {
            StaticWord::I64(_) => out.push_str("i64"),
            StaticWord::Ptr(_) => out.push_str("ptr"),
        }
    }
    out.push_str(" } { ");
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        match word {
            StaticWord::I64(value) => {
                out.push_str("i64 ");
                out.push_str(value);
            }
            StaticWord::Ptr(symbol) => {
                out.push_str("ptr @");
                out.push_str(symbol);
            }
        }
    }
    out.push_str(" }\n");
}

/// Format a non-static SSA value as an LLVM IR operand.
pub(crate) fn fmt_value(val: &SsaValue) -> String {
    match val {
        SsaValue::Reg(name) => format!("%{}", mangle(name)),
        SsaValue::Lit(lit) => fmt_lit(lit),
        SsaValue::Global(name) => {
            panic!("internal codegen error: global value `{name}` used without SSA materialization")
        }
        SsaValue::GlobalClosure(_) => {
            panic!("internal codegen error: static closure used without PIE materialization")
        }
    }
}

/// Format a non-static literal as an LLVM IR constant.
pub(crate) fn fmt_lit(lit: &DfLit) -> String {
    match lit {
        DfLit::Bool(b) => {
            if *b {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        DfLit::Int(n) => n.to_string(),
        DfLit::Float(f) => {
            // Store the IEEE-754 bit pattern as a *decimal* i64 literal. LLVM parses a
            // 16-hex-digit `0x...` as a double constant, which is invalid in i64 position.
            (f.to_bits() as i64).to_string()
        }
        DfLit::Posit(literal) => {
            if literal.spec.nbits == 32 {
                (literal.bits as u32).to_string()
            } else {
                (literal.bits as i64).to_string()
            }
        }
        DfLit::Text(_) | DfLit::Atom(_) => {
            panic!("internal codegen error: static literal used without PIE materialization")
        }
    }
}

pub(crate) fn emit_symbol_ptr_to_i64(out: &mut String, tmp: &mut u64, symbol: &str) -> String {
    let name = alloc_tmp(tmp);
    out.push_str(&format!("  {name} = ptrtoint ptr @{symbol} to i64\n"));
    name
}

pub(crate) fn emit_value_operand(out: &mut String, tmp: &mut u64, value: &SsaValue) -> String {
    match value {
        SsaValue::GlobalClosure(name) => {
            emit_symbol_ptr_to_i64(out, tmp, &closure_global_name(name))
        }
        SsaValue::Lit(DfLit::Text(s)) => {
            let symbol = format!("zutai.text.{}", str_hash(s));
            emit_symbol_ptr_to_i64(out, tmp, &symbol)
        }
        SsaValue::Lit(DfLit::Atom(s)) => {
            let symbol = format!("zutai.atom.{}", str_hash(s));
            emit_symbol_ptr_to_i64(out, tmp, &symbol)
        }
        _ => fmt_value(value),
    }
}

pub(crate) fn fmt_phi_value(value: &SsaValue) -> String {
    match value {
        SsaValue::GlobalClosure(_) | SsaValue::Lit(DfLit::Text(_) | DfLit::Atom(_)) => panic!(
            "internal codegen error: PIE static value reached phi without SSA materialization"
        ),
        SsaValue::Global(name) => {
            panic!(
                "internal codegen error: global value `{name}` reached phi without SSA materialization"
            )
        }
        _ => fmt_value(value),
    }
}

/// FNV-1a hash for naming global string constants.
pub(crate) fn str_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

/// LLVM IR binary opcode name for `i64`.
pub(crate) fn builtin_ir_op(op: &DfBuiltinOp) -> &'static str {
    match op {
        DfBuiltinOp::Add => "add",
        DfBuiltinOp::Sub => "sub",
        DfBuiltinOp::Mul => "mul",
        DfBuiltinOp::Div => "sdiv",
        DfBuiltinOp::Eq => "icmp eq",
        DfBuiltinOp::Ne => "icmp ne",
        DfBuiltinOp::Lt => "icmp slt",
        DfBuiltinOp::Le => "icmp sle",
        DfBuiltinOp::Gt => "icmp sgt",
        DfBuiltinOp::Ge => "icmp sge",
        DfBuiltinOp::And => "and",
        DfBuiltinOp::Or => "or",
        DfBuiltinOp::Posit { .. } => unreachable!("posit builtins lower through helper calls"),
    }
}

/// Whether a builtin produces an `i1` (comparison) result.
pub(crate) fn builtin_is_cmp(op: &DfBuiltinOp) -> bool {
    matches!(
        op,
        DfBuiltinOp::Eq
            | DfBuiltinOp::Ne
            | DfBuiltinOp::Lt
            | DfBuiltinOp::Le
            | DfBuiltinOp::Gt
            | DfBuiltinOp::Ge
            | DfBuiltinOp::Posit {
                op: DfPositOp::Eq
                    | DfPositOp::Ne
                    | DfPositOp::Lt
                    | DfPositOp::Le
                    | DfPositOp::Gt
                    | DfPositOp::Ge,
                ..
            }
    )
}
