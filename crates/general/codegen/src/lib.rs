//! LLVM IR text emission from Zutai SSA modules.
//!
//! Produces `.ll` files targeting `x86_64-unknown-linux-gnu`.
//! All Zutai values are represented as `i64` for v0 simplicity:
//! integers are stored directly, booleans as 0/1, and compound
//! values (records, tuples, lists, closures, text) are heap-allocated
//! with their pointer cast to `i64`.

use zutai_ssa::*;

// ── Public API ─────────────────────────────────────────────────────────────────

/// Emit a complete LLVM IR `.ll` file from an SSA module.
pub fn emit_llvm(module: &SsaModule) -> String {
    let mut out = String::with_capacity(8192);
    emit_preamble(&mut out);
    emit_type_decls(&mut out);
    emit_runtime_decls(&mut out);
    emit_posit_runtime_decls(module, &mut out);
    collect_and_emit_constants(module, &mut out);
    emit_static_closures(&mut out, module);

    let all_funcs = collect_functions(module);
    for func in &all_funcs {
        emit_func_decl(&mut out, func);
    }
    for func in &all_funcs {
        emit_func_def(&mut out, func);
    }

    emit_main(&mut out, &module.entry.name, &module.entry_ty);
    out
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// All functions reachable from the module (entry + every decl).
fn collect_functions(module: &SsaModule) -> Vec<&SsaFunc> {
    let mut funcs = Vec::new();
    funcs.push(&module.entry);
    for decl in &module.decls {
        match decl {
            SsaDecl::Func(f) => funcs.push(f),
            SsaDecl::RecGroup(group) => funcs.extend(group),
        }
    }
    funcs
}

/// Sanitise a Zutai identifier into a valid LLVM IR name.
fn mangle(name: &str) -> String {
    name.replace(['-', '.', '='], "_")
        .replace('?', "_Q")
        .replace('!', "_B")
        .replace('@', "_at_")
}

/// D-0003 closure object tag (matches `TAG_CLOSURE` in `zutai-rt`).
const CLOSURE_TAG: u64 = 7;

/// Pack a closure header word: low byte = tag, next bits = capture count.
fn closure_header(ncaps: usize) -> u64 {
    ((ncaps as u64) << 8) | CLOSURE_TAG
}

/// Global symbol name of the static closure object for a top-level function.
fn closure_global_name(name: &str) -> String {
    format!("zutai.closure.{}", mangle(name))
}

/// Format an SSA value as an LLVM IR operand (appends to `out`).
fn fmt_value(val: &SsaValue, out: &mut String) {
    match val {
        SsaValue::Reg(name) => {
            out.push('%');
            out.push_str(&mangle(name));
        }
        SsaValue::Lit(lit) => fmt_lit(lit, out),
        SsaValue::Global(name) => {
            out.push('@');
            out.push_str(&mangle(name));
        }
        SsaValue::GlobalClosure(name) => {
            out.push_str("ptrtoint (ptr @");
            out.push_str(&closure_global_name(name));
            out.push_str(" to i64)");
        }
    }
}

/// Format a literal as an LLVM IR constant (appends to `out`).
fn fmt_lit(lit: &DfLit, out: &mut String) {
    match lit {
        DfLit::Bool(b) => out.push_str(if *b { "1" } else { "0" }),
        DfLit::Int(n) => out.push_str(&n.to_string()),
        DfLit::Float(f) => {
            // Encode float as its IEEE 754 double bit pattern in an i64.
            // This lets us store the exact float value in our uniform i64 type.
            let bits = f.to_bits();
            out.push_str(&format!("0x{:016x}", bits));
        }
        DfLit::Posit(literal) => {
            if literal.spec.nbits == 32 {
                out.push_str(&format!("0x00000000{:08x}", literal.bits as u32));
            } else {
                out.push_str(&format!("0x{:016x}", literal.bits));
            }
        }
        DfLit::Text(s) => {
            out.push_str("@zutai.text.");
            out.push_str(&str_hash(s));
        }
        DfLit::Atom(s) => {
            out.push_str("@zutai.atom.");
            out.push_str(&str_hash(s));
        }
    }
}

/// FNV-1a hash for naming global string constants.
fn str_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

/// Deterministic atom tag from atom name.
fn atom_tag(s: &str) -> u64 {
    let mut h: u64 = 0x9e3779b97f4a7c15;
    for b in s.bytes() {
        h = h.wrapping_mul(0x100000001b3) ^ (b as u64);
    }
    h
}

/// LLVM IR binary opcode name for `i64`.
fn builtin_ir_op(op: &DfBuiltinOp) -> &'static str {
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
fn builtin_is_cmp(op: &DfBuiltinOp) -> bool {
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

// ── Preamble & declarations ────────────────────────────────────────────────────

fn emit_preamble(out: &mut String) {
    out.push_str("target triple = \"x86_64-unknown-linux-gnu\"\n");
    out.push_str("target datalayout = \"e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128\"\n\n");
}

fn emit_type_decls(out: &mut String) {
    out.push_str("; Zutai runtime types (v0: all values are i64)\n\n");
}

fn emit_runtime_decls(out: &mut String) {
    out.push_str("; ── Runtime helpers ───────────────────────────────────────────────\n\n");

    // Allocation
    out.push_str("declare i64 @zutai.alloc(i64)\n");
    out.push_str("declare void @zutai.free(i64)\n");

    // Printing
    out.push_str("declare void @zutai.print_i64(i64)\n");
    out.push_str("declare void @zutai.print_text(i64)\n");
    out.push_str("declare void @zutai.print_bool(i64)\n");
    out.push_str("declare void @zutai.print_float(i64)\n");
    out.push_str("declare void @zutai.print_posit(i64, i64, i64)\n");

    // Record operations
    out.push_str("declare i64 @zutai.record_new(i64)\n");
    out.push_str("declare void @zutai.record_set(i64, i64, i64)\n");
    out.push_str("declare i64 @zutai.record_get(i64, i64)\n");
    out.push_str("declare i64 @zutai.record_update(i64, i64, i64)\n");

    // Tuple operations
    out.push_str("declare i64 @zutai.tuple_new(i64)\n");
    out.push_str("declare void @zutai.tuple_set(i64, i64, i64)\n");
    out.push_str("declare i64 @zutai.tuple_get(i64, i64)\n");

    // List operations
    out.push_str("declare i64 @zutai.list_cons(i64, i64)\n");
    out.push_str("declare i64 @zutai.list_nil()\n");

    // Optional/Maybe operations
    out.push_str("declare i64 @zutai.coalesce(i64, i64)\n");
    // Variant operations
    out.push_str("declare i64 @zutai.variant_new(i64, i64)\n");
    out.push_str("declare i64 @zutai.variant_tag(i64)\n");
    out.push_str("declare i64 @zutai.variant_value(i64)\n");

    // Text operations
    out.push_str("declare i64 @zutai.text_from_global(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_concat(i64, i64)\n");

    // C stdlib
    out.push_str("declare i64 @exit(i64)\n\n");
}
fn emit_posit_runtime_decls(module: &SsaModule, out: &mut String) {
    let mut pairs: Vec<(u8, u8, DfPositOp)> = Vec::new();
    for func in collect_functions(module) {
        for block in &func.blocks {
            for instr in &block.instructions {
                if let SsaOp::Builtin {
                    op: DfBuiltinOp::Posit { op, spec },
                    ..
                } = instr.op
                {
                    let pair = (spec.nbits, spec.es, op);
                    if !pairs.contains(&pair) {
                        pairs.push(pair);
                    }
                }
            }
        }
    }
    if pairs.is_empty() {
        return;
    }

    out.push_str("; ── Posit runtime helpers ─────────────────────────────────────────\n\n");
    for (nbits, es, op) in pairs {
        let ret = if posit_op_is_cmp(op) {
            "i1"
        } else if nbits == 32 {
            "i32"
        } else {
            "i64"
        };
        let arg = if nbits == 32 { "i32" } else { "i64" };
        out.push_str(&format!(
            "declare {ret} @zutai.posit{nbits}e{es}.{}({arg}, {arg})\n",
            posit_op_name(op)
        ));
    }
    out.push('\n');
}

fn posit_op_name(op: DfPositOp) -> &'static str {
    match op {
        DfPositOp::Add => "add",
        DfPositOp::Sub => "sub",
        DfPositOp::Mul => "mul",
        DfPositOp::Div => "div",
        DfPositOp::Eq => "eq",
        DfPositOp::Ne => "ne",
        DfPositOp::Lt => "lt",
        DfPositOp::Le => "le",
        DfPositOp::Gt => "gt",
        DfPositOp::Ge => "ge",
    }
}

fn posit_op_is_cmp(op: DfPositOp) -> bool {
    matches!(
        op,
        DfPositOp::Eq
            | DfPositOp::Ne
            | DfPositOp::Lt
            | DfPositOp::Le
            | DfPositOp::Gt
            | DfPositOp::Ge
    )
}

fn emit_func_decl(out: &mut String, func: &SsaFunc) {
    let name = mangle(&func.name);
    let params = func
        .params
        .iter()
        .map(|p| format!("i64 %{}", mangle(p)))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("declare i64 @{}({})\n", name, params));
}

/// Emit the static empty-capture closure object for every top-level function so
/// that `GlobalClosure(name)` resolves to `@zutai.closure.<name>` (D-0003).
fn emit_static_closures(out: &mut String, module: &SsaModule) {
    if module.closure_exports.is_empty() {
        return;
    }
    out.push_str("; ── Top-level closures (D-0003) ───────────────────────────────────\n\n");
    for name in &module.closure_exports {
        out.push_str(&format!(
            "@{} = internal constant [2 x i64] [i64 {}, i64 ptrtoint (ptr @{} to i64)]\n",
            closure_global_name(name),
            closure_header(0),
            mangle(name)
        ));
    }
    out.push('\n');
}

// ── Text / Atom constant emission ─────────────────────────────────────────────

enum Constant {
    Text(String),
    Atom(String),
}

fn collect_and_emit_constants(module: &SsaModule, out: &mut String) {
    let mut constants: Vec<Constant> = Vec::new();
    collect_from_func(&module.entry, &mut constants);
    for decl in &module.decls {
        match decl {
            SsaDecl::Func(f) => collect_from_func(f, &mut constants),
            SsaDecl::RecGroup(group) => {
                for f in group {
                    collect_from_func(f, &mut constants);
                }
            }
        }
    }
    if constants.is_empty() {
        return;
    }
    out.push_str("; ── Global constants ───────────────────────────────────────────\n\n");
    for c in &constants {
        match c {
            Constant::Text(s) => {
                let hash = str_hash(s);
                let esc = llvm_string_bytes(s);
                out.push_str(&format!(
                    "@zutai.text.data.{} = private unnamed_addr constant [{} x i8] c\"{}\"\n",
                    hash, esc.len, esc.escaped
                ));
                out.push_str(&format!(
                    "@zutai.text.{} = global i64 ptrtoint ([{} x i8]* @zutai.text.data.{} to i64)\n",
                    hash, esc.len, hash
                ));
            }
            Constant::Atom(s) => {
                let hash = str_hash(s);
                let esc = llvm_string_bytes(s);
                let tag = atom_tag(s);
                out.push_str(&format!(
                    "@zutai.atom.data.{} = private unnamed_addr constant [{} x i8] c\"{}\"\n",
                    hash, esc.len, esc.escaped
                ));
                // Atom: represented as i64 tag value for v0.
                // The string data is available at @zutai.atom.data.HASH for the runtime.
                out.push_str(&format!("@zutai.atom.{} = global i64 {}\n", hash, tag));
            }
        }
    }
    out.push('\n');
}

fn collect_from_func(func: &SsaFunc, constants: &mut Vec<Constant>) {
    for block in &func.blocks {
        for instr in &block.instructions {
            collect_from_op(&instr.op, constants);
        }
        collect_from_terminator(&block.terminator, constants);
    }
}

fn collect_from_op(op: &SsaOp, constants: &mut Vec<Constant>) {
    match op {
        SsaOp::ApplyClosure { closure, arg } => {
            collect_from_value(closure, constants);
            collect_from_value(arg, constants);
        }
        SsaOp::MakeClosure { code: _, captures } => {
            for c in captures {
                collect_from_value(c, constants);
            }
        }
        SsaOp::LoadCapture { closure, index: _ } => collect_from_value(closure, constants),
        SsaOp::TyApp { .. } => {}
        SsaOp::Record { fields } => {
            for (_, v) in fields {
                collect_from_value(v, constants);
            }
        }
        SsaOp::RecordUpdate { base, updates } => {
            collect_from_value(base, constants);
            for (_, value) in updates {
                collect_from_value(value, constants);
            }
        }
        SsaOp::Tuple { items } => {
            for item in items {
                match item {
                    SsaTupleItem::Named { name: _, value } | SsaTupleItem::Positional(value) => {
                        collect_from_value(value, constants)
                    }
                }
            }
        }
        SsaOp::List { elems } => {
            for v in elems {
                collect_from_value(v, constants);
            }
        }
        SsaOp::Select { base, field: _ } => collect_from_value(base, constants),
        SsaOp::Variant { tag: _, value } => collect_from_value(value, constants),
        SsaOp::Builtin { op: _, lhs, rhs } => {
            collect_from_value(lhs, constants);
            collect_from_value(rhs, constants);
        }
        SsaOp::Coalesce { value, fallback } => {
            collect_from_value(value, constants);
            collect_from_value(fallback, constants);
        }
        SsaOp::Error => {}
        SsaOp::Phi { branches } => {
            for (_, v) in branches {
                collect_from_value(v, constants);
            }
        }
        SsaOp::MatchDiscriminant { scrutinee } => collect_from_value(scrutinee, constants),
    }
}

fn collect_from_value(val: &SsaValue, constants: &mut Vec<Constant>) {
    match val {
        SsaValue::Lit(DfLit::Text(s)) => constants.push(Constant::Text(s.clone())),
        SsaValue::Lit(DfLit::Atom(s)) => constants.push(Constant::Atom(s.clone())),
        _ => {}
    }
}

fn collect_from_terminator(term: &SsaTerminator, constants: &mut Vec<Constant>) {
    match term {
        SsaTerminator::Return(v) => collect_from_value(v, constants),
        SsaTerminator::Jump(_) => {}
        SsaTerminator::Branch { cond, .. } => collect_from_value(cond, constants),
    }
}

struct EscapedString {
    len: usize,
    escaped: String,
}

/// Escape a Rust string into the LLVM IR `c"..."` byte-literal format,
/// including a null terminator.
fn llvm_string_bytes(s: &str) -> EscapedString {
    let mut escaped = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'\\' => escaped.push_str(r"\\"),
            b'"' => escaped.push_str("\\\""),
            0x20..=0x7e => escaped.push(b as char),
            _ => escaped.push_str(&format!("\\{:02x}", b)),
        }
    }
    escaped.push_str("\\00"); // null terminator
    EscapedString {
        len: s.len() + 1,
        escaped,
    }
}

// ── Function definitions ───────────────────────────────────────────────────────

fn emit_func_def(out: &mut String, func: &SsaFunc) {
    let name = mangle(&func.name);
    let params = func
        .params
        .iter()
        .map(|p| format!("i64 %{}", mangle(p)))
        .collect::<Vec<_>>()
        .join(", ");

    out.push_str(&format!("define i64 @{}({}) {{\n", name, params));

    let mut tmp = 0u64;

    for (block_idx, block) in func.blocks.iter().enumerate() {
        let label = mangle(&block.label);
        if block_idx == 0 {
            out.push_str("entry:\n");
        } else {
            out.push_str(&format!("{}:\n", label));
        }

        for instr in &block.instructions {
            emit_instr(out, instr, &mut tmp);
        }
        emit_terminator(out, &block.terminator, &mut tmp);
    }

    out.push_str("}\n\n");
}

fn emit_posit_instr(
    out: &mut String,
    dest: &str,
    op: DfPositOp,
    spec: (u8, u8),
    lhs: &SsaValue,
    rhs: &SsaValue,
    tmp: &mut u64,
) {
    let (nbits, es) = spec;
    let helper = format!("@zutai.posit{nbits}e{es}.{}", posit_op_name(op));
    match (nbits, posit_op_is_cmp(op)) {
        (32, false) => {
            let lhs32 = alloc_tmp(tmp);
            let rhs32 = alloc_tmp(tmp);
            let call = alloc_tmp(tmp);
            out.push_str(&format!("  {lhs32} = trunc i64 "));
            fmt_value(lhs, out);
            out.push_str(" to i32\n");
            out.push_str(&format!("  {rhs32} = trunc i64 "));
            fmt_value(rhs, out);
            out.push_str(" to i32\n");
            out.push_str(&format!(
                "  {call} = call i32 {helper}(i32 {lhs32}, i32 {rhs32})\n"
            ));
            out.push_str(&format!("  %{dest} = zext i32 {call} to i64\n"));
        }
        (32, true) => {
            let lhs32 = alloc_tmp(tmp);
            let rhs32 = alloc_tmp(tmp);
            let call = alloc_tmp(tmp);
            out.push_str(&format!("  {lhs32} = trunc i64 "));
            fmt_value(lhs, out);
            out.push_str(" to i32\n");
            out.push_str(&format!("  {rhs32} = trunc i64 "));
            fmt_value(rhs, out);
            out.push_str(" to i32\n");
            out.push_str(&format!(
                "  {call} = call i1 {helper}(i32 {lhs32}, i32 {rhs32})\n"
            ));
            out.push_str(&format!("  %{dest} = zext i1 {call} to i64\n"));
        }
        (64, false) => {
            out.push_str(&format!("  %{dest} = call i64 {helper}(i64 "));
            fmt_value(lhs, out);
            out.push_str(", i64 ");
            fmt_value(rhs, out);
            out.push_str(")\n");
        }
        (64, true) => {
            let call = alloc_tmp(tmp);
            out.push_str(&format!("  {call} = call i1 {helper}(i64 "));
            fmt_value(lhs, out);
            out.push_str(", i64 ");
            fmt_value(rhs, out);
            out.push_str(")\n");
            out.push_str(&format!("  %{dest} = zext i1 {call} to i64\n"));
        }
        _ => unreachable!("invalid posit width"),
    }
}

fn emit_instr(out: &mut String, instr: &SsaInstr, tmp: &mut u64) {
    let dest = mangle(&instr.dest);

    match &instr.op {
        // ── ApplyClosure (D-0003 uniform closure application) ───────────────
        SsaOp::ApplyClosure { closure, arg } => {
            let cptr = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 ", cptr));
            fmt_value(closure, out);
            out.push_str(" to ptr\n");
            let code_slot = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {} = getelementptr i64, ptr {}, i64 1\n",
                code_slot, cptr
            ));
            let code = alloc_tmp(tmp);
            out.push_str(&format!("  {} = load i64, ptr {}\n", code, code_slot));
            let fnptr = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", fnptr, code));
            out.push_str(&format!("  %{} = call i64 {}(i64 ", dest, fnptr));
            fmt_value(closure, out);
            out.push_str(", i64 ");
            fmt_value(arg, out);
            out.push_str(")\n");
        }

        // ── MakeClosure (heap closure allocation) ───────────────────────────
        SsaOp::MakeClosure { code, captures } => {
            let bytes = (2 + captures.len()) * 8;
            let raw = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {} = call i64 @zutai.alloc(i64 {})\n",
                raw, bytes
            ));
            let base = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", base, raw));
            out.push_str(&format!(
                "  store i64 {}, ptr {}\n",
                closure_header(captures.len()),
                base
            ));
            let code_slot = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {} = getelementptr i64, ptr {}, i64 1\n",
                code_slot, base
            ));
            out.push_str(&format!(
                "  store i64 ptrtoint (ptr @{} to i64), ptr {}\n",
                mangle(code),
                code_slot
            ));
            for (index, cap) in captures.iter().enumerate() {
                let slot = alloc_tmp(tmp);
                out.push_str(&format!(
                    "  {} = getelementptr i64, ptr {}, i64 {}\n",
                    slot,
                    base,
                    2 + index
                ));
                out.push_str("  store i64 ");
                fmt_value(cap, out);
                out.push_str(&format!(", ptr {}\n", slot));
            }
            out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, raw));
        }

        // ── LoadCapture (read a capture from the enclosing closure) ─────────
        SsaOp::LoadCapture { closure, index } => {
            let cptr = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 ", cptr));
            fmt_value(closure, out);
            out.push_str(" to ptr\n");
            let slot = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {} = getelementptr i64, ptr {}, i64 {}\n",
                slot,
                cptr,
                2 + index
            ));
            out.push_str(&format!("  %{} = load i64, ptr {}\n", dest, slot));
        }

        // ── TyApp (erased) ─────────────────────────────────────────────────
        SsaOp::TyApp { poly, ty_args: _ } => {
            // Type application is erased at runtime; copy the value.
            out.push_str(&format!("  %{} = add i64 ", dest));
            fmt_value(poly, out);
            out.push_str(", 0\n");
        }

        // ── Record ─────────────────────────────────────────────────────────
        SsaOp::Record { fields } => {
            let count = fields.len() as u64;
            out.push_str(&format!(
                "  %{}.rec = call i64 @zutai.record_new(i64 {})\n",
                dest, count
            ));
            for (idx, (_, value)) in fields.iter().enumerate() {
                out.push_str(&format!(
                    "  call void @zutai.record_set(i64 %{}.rec, i64 {}, i64 ",
                    dest, idx
                ));
                fmt_value(value, out);
                out.push_str(")\n");
            }
            out.push_str(&format!("  %{} = add i64 %{}.rec, 0\n", dest, dest));
        }

        // ── Record update ───────────────────────────────────────────────────
        SsaOp::RecordUpdate { base, updates } => {
            if updates.is_empty() {
                out.push_str(&format!("  %{} = add i64 ", dest));
                fmt_value(base, out);
                out.push_str(", 0\n");
            } else {
                let mut prev = String::new();
                fmt_value(base, &mut prev);
                for (idx, (field, value)) in updates.iter().enumerate() {
                    let tmp_name = format!("%{}.upd{}", dest, idx);
                    let field_key = u64::from_str_radix(&str_hash(field), 16).unwrap_or(0);
                    out.push_str(&format!(
                        "  {} = call i64 @zutai.record_update(i64 {}, i64 {}, i64 ",
                        tmp_name, prev, field_key
                    ));
                    fmt_value(value, out);
                    out.push_str(")\n");
                    prev = tmp_name;
                }
                out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, prev));
            }
        }

        // ── Tuple ──────────────────────────────────────────────────────────
        SsaOp::Tuple { items } => {
            let count = items.len() as u64;
            out.push_str(&format!(
                "  %{}.tup = call i64 @zutai.tuple_new(i64 {})\n",
                dest, count
            ));
            for (idx, item) in items.iter().enumerate() {
                let value = match item {
                    SsaTupleItem::Named { name: _, value } | SsaTupleItem::Positional(value) => {
                        value
                    }
                };
                out.push_str(&format!(
                    "  call void @zutai.tuple_set(i64 %{}.tup, i64 {}, i64 ",
                    dest, idx
                ));
                fmt_value(value, out);
                out.push_str(")\n");
            }
            out.push_str(&format!("  %{} = add i64 %{}.tup, 0\n", dest, dest));
        }

        // ── List ────────────────────────────────────────────────────────────
        SsaOp::List { elems } => {
            if elems.is_empty() {
                out.push_str(&format!("  %{} = call i64 @zutai.list_nil()\n", dest));
            } else {
                // Build from right to left: nil, then cons each element.
                let nil_tmp = alloc_tmp(tmp);
                out.push_str(&format!("  {} = call i64 @zutai.list_nil()\n", nil_tmp));

                let mut prev = nil_tmp;
                for (i, elem) in elems.iter().enumerate().rev() {
                    let cons_tmp = if i == 0 {
                        format!("%{}", dest)
                    } else {
                        alloc_tmp(tmp)
                    };
                    out.push_str(&format!("  {} = call i64 @zutai.list_cons(i64 ", cons_tmp));
                    fmt_value(elem, out);
                    out.push_str(&format!(", i64 {})\n", prev));
                    prev = cons_tmp;
                }
            }
        }

        // ── Select ──────────────────────────────────────────────────────────
        SsaOp::Select { base, field } => {
            let field_key = u64::from_str_radix(&str_hash(field), 16).unwrap_or(0);
            out.push_str(&format!("  %{} = call i64 @zutai.record_get(i64 ", dest));
            fmt_value(base, out);
            out.push_str(&format!(", i64 {})\n", field_key));
        }

        // ── Variant ─────────────────────────────────────────────────────────
        SsaOp::Variant { tag, value } => {
            let tag_val = atom_tag(tag);
            out.push_str(&format!(
                "  %{} = call i64 @zutai.variant_new(i64 {}, i64 ",
                dest, tag_val
            ));
            fmt_value(value, out);
            out.push_str(")\n");
        }

        // ── Builtin ─────────────────────────────────────────────────────────
        SsaOp::Builtin {
            op: DfBuiltinOp::Posit { op, spec },
            lhs,
            rhs,
        } => {
            emit_posit_instr(out, &dest, *op, (spec.nbits, spec.es), lhs, rhs, tmp);
        }
        SsaOp::Builtin { op, lhs, rhs } => {
            if builtin_is_cmp(op) {
                // Comparisons yield i1; zext to i64.
                let cmp_tmp = alloc_tmp(tmp);
                out.push_str(&format!("  {} = {} i64 ", cmp_tmp, builtin_ir_op(op)));
                fmt_value(lhs, out);
                out.push_str(", ");
                fmt_value(rhs, out);
                out.push('\n');
                out.push_str(&format!("  %{} = zext i1 {} to i64\n", dest, cmp_tmp));
            } else {
                // Arithmetic / bitwise on i64.
                out.push_str(&format!("  %{} = {} i64 ", dest, builtin_ir_op(op)));
                fmt_value(lhs, out);
                out.push_str(", ");
                fmt_value(rhs, out);
                out.push('\n');
            }
        }

        // ── Coalesce ────────────────────────────────────────────────────────
        SsaOp::Coalesce { value, fallback } => {
            // @zutai.coalesce unwraps one Optional or Maybe layer:
            // #none/#absent choose fallback; #some (x)/#present (x) return x.
            out.push_str(&format!("  %{} = call i64 @zutai.coalesce(i64 ", dest));
            fmt_value(value, out);
            out.push_str(", i64 ");
            fmt_value(fallback, out);
            out.push_str(")\n");
        }

        // ── Error ───────────────────────────────────────────────────────────
        SsaOp::Error => {
            out.push_str(&format!("  %{} = add i64 0, 0\n", dest));
        }

        // ── Phi ─────────────────────────────────────────────────────────────
        SsaOp::Phi { branches } => {
            out.push_str(&format!("  %{} = phi i64 ", dest));
            for (i, (label, val)) in branches.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push('[');
                fmt_value(val, out);
                out.push_str(&format!(", %{}]", mangle(label)));
            }
            out.push('\n');
        }

        // ── MatchDiscriminant ───────────────────────────────────────────────
        SsaOp::MatchDiscriminant { scrutinee } => {
            out.push_str(&format!("  %{} = call i64 @zutai.variant_tag(i64 ", dest));
            fmt_value(scrutinee, out);
            out.push_str(")\n");
        }
    }
}

fn emit_terminator(out: &mut String, term: &SsaTerminator, tmp: &mut u64) {
    match term {
        SsaTerminator::Return(val) => {
            out.push_str("  ret i64 ");
            fmt_value(val, out);
            out.push('\n');
        }
        SsaTerminator::Jump(label) => {
            out.push_str(&format!("  br label %{}\n", mangle(label)));
        }
        SsaTerminator::Branch {
            cond,
            then_label,
            else_label,
        } => {
            // Emit: %cond_tmp = icmp ne i64 <cond>, 0
            //       br i1 %cond_tmp, label %then, label %else
            let cond_tmp = alloc_tmp(tmp);
            out.push_str(&format!("  {} = icmp ne i64 ", cond_tmp));
            fmt_value(cond, out);
            out.push_str(", 0\n");
            out.push_str(&format!(
                "  br i1 {}, label %{}, label %{}\n",
                cond_tmp,
                mangle(then_label),
                mangle(else_label)
            ));
        }
    }
}

fn alloc_tmp(tmp: &mut u64) -> String {
    let id = *tmp;
    *tmp += 1;
    format!("%_tmp.{}", id)
}

// ── @main ─────────────────────────────────────────────────────────────────────

fn emit_main(out: &mut String, entry_name: &str, entry_ty: &DfTy) {
    let entry = mangle(entry_name);
    out.push_str(&format!(
        "define i32 @main() {{\n  %result = call i64 @{}()\n",
        entry
    ));
    match entry_ty {
        DfTy::Posit(spec) => out.push_str(&format!(
            "  call void @zutai.print_posit(i64 %result, i64 {}, i64 {})\n",
            spec.nbits, spec.es
        )),
        _ => out.push_str("  call void @zutai.print_i64(i64 %result)\n"),
    }
    out.push_str("  ret i32 0\n}\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use zutai_syntax::posit::{PositLiteral, PositSpec};

    fn posit_module(spec: PositSpec, op: DfPositOp, entry_ty: DfTy) -> SsaModule {
        SsaModule {
            decls: Vec::new(),
            entry: SsaFunc {
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
            closure_exports: Vec::new(),
        }
    }

    #[test]
    fn coalesce_emits_runtime_helper_call() {
        let module = SsaModule {
            decls: Vec::new(),
            entry: SsaFunc {
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
            entry_ty: DfTy::Int,
            closure_exports: Vec::new(),
        };

        let llvm = emit_llvm(&module);
        assert!(llvm.contains("call i64 @zutai.coalesce"));
        assert!(!llvm.contains("icmp ne i64"), "{llvm}");
    }

    #[test]
    fn record_update_emits_runtime_helper_call() {
        let module = SsaModule {
            decls: Vec::new(),
            entry: SsaFunc {
                name: "__entry".to_string(),
                params: Vec::new(),
                blocks: vec![SsaBlock {
                    label: "entry".to_string(),
                    instructions: vec![SsaInstr {
                        dest: "result".to_string(),
                        op: SsaOp::RecordUpdate {
                            base: SsaValue::Reg("base".to_string()),
                            updates: vec![("port".to_string(), SsaValue::Lit(DfLit::Int(8080)))],
                        },
                    }],
                    terminator: SsaTerminator::Return(SsaValue::Reg("result".to_string())),
                }],
            },
            entry_ty: DfTy::Int,
            closure_exports: Vec::new(),
        };

        let llvm = emit_llvm(&module);
        assert!(llvm.contains("declare i64 @zutai.record_update"));
        assert!(llvm.contains("call i64 @zutai.record_update"));
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
        let module = SsaModule {
            decls: vec![SsaDecl::Func(SsaFunc {
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
            entry: SsaFunc {
                name: "__entry".to_string(),
                params: Vec::new(),
                blocks: vec![SsaBlock {
                    label: "entry".to_string(),
                    instructions: Vec::new(),
                    terminator: SsaTerminator::Return(SsaValue::Lit(DfLit::Int(0))),
                }],
            },
            entry_ty: DfTy::Int,
            closure_exports: vec!["inc".to_string()],
        };

        let llvm = emit_llvm(&module);
        assert!(
            llvm.contains(
                "@zutai.closure.inc = internal constant [2 x i64] [i64 7, i64 ptrtoint (ptr @inc to i64)]"
            ),
            "{llvm}"
        );
    }

    #[test]
    fn closure_apply_loads_code_and_passes_self() {
        let module = SsaModule {
            decls: Vec::new(),
            entry: SsaFunc {
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
            entry_ty: DfTy::Int,
            closure_exports: Vec::new(),
        };

        let llvm = emit_llvm(&module);
        assert!(llvm.contains("getelementptr i64, ptr"), "{llvm}");
        assert!(llvm.contains("load i64, ptr"), "{llvm}");
        // Code pointer is called indirectly with (self, arg).
        assert!(
            llvm.contains("call i64 %"),
            "indirect call expected: {llvm}"
        );
        // Legacy direct/raw call shapes are gone.
        assert!(!llvm.contains("call i64 @inc(i64 41)"), "{llvm}");
        assert!(!llvm.contains("to i64 (i64)*"), "{llvm}");
    }

    #[test]
    fn capturing_lambda_allocates_heap_closure() {
        let module = SsaModule {
            decls: Vec::new(),
            entry: SsaFunc {
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
            entry_ty: DfTy::Int,
            closure_exports: Vec::new(),
        };

        let llvm = emit_llvm(&module);
        // (2 + 1 capture) * 8 bytes = 24.
        assert!(llvm.contains("call i64 @zutai.alloc(i64 24)"), "{llvm}");
        // Header for one capture: (1 << 8) | 7 = 263.
        assert!(llvm.contains("store i64 263,"), "{llvm}");
        // Capture stored at slot 2.
        assert!(llvm.contains(", i64 2\n"), "slot-2 gep expected: {llvm}");
        assert!(
            llvm.contains("store i64 10,"),
            "capture value stored: {llvm}"
        );
    }
}
