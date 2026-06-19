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
    collect_and_emit_constants(module, &mut out);

    let all_funcs = collect_functions(module);
    for func in &all_funcs {
        emit_func_decl(&mut out, func);
    }
    for func in &all_funcs {
        emit_func_def(&mut out, func);
    }

    emit_main(&mut out, &module.entry.name);
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

    // Record operations
    out.push_str("declare i64 @zutai.record_new(i64)\n");
    out.push_str("declare void @zutai.record_set(i64, i64, i64)\n");
    out.push_str("declare i64 @zutai.record_get(i64, i64)\n");

    // Tuple operations
    out.push_str("declare i64 @zutai.tuple_new(i64)\n");
    out.push_str("declare void @zutai.tuple_set(i64, i64, i64)\n");
    out.push_str("declare i64 @zutai.tuple_get(i64, i64)\n");

    // List operations
    out.push_str("declare i64 @zutai.list_cons(i64, i64)\n");
    out.push_str("declare i64 @zutai.list_nil()\n");

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
        SsaOp::Call { func: _, arg } => collect_from_value(arg, constants),
        SsaOp::TyApp { .. } => {}
        SsaOp::Record { fields } => {
            for (_, v) in fields {
                collect_from_value(v, constants);
            }
        }
        SsaOp::Tuple { items } => {
            for item in items {
                match item {
                    SsaTupleItem::Named { name: _, value }
                    | SsaTupleItem::Positional(value) => collect_from_value(value, constants),
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
    // Also check the func value in Call for global refs that might contain literals.
    if let SsaOp::Call { func, .. } = op {
        collect_from_value(func, constants);
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

fn emit_instr(out: &mut String, instr: &SsaInstr, tmp: &mut u64) {
    let dest = mangle(&instr.dest);

    match &instr.op {
        // ── Call ────────────────────────────────────────────────────────────
        SsaOp::Call { func, arg } => match func {
            SsaValue::Global(fname) => {
                let fname = mangle(fname);
                out.push_str(&format!("  %{} = call i64 @{}(", dest, fname));
                fmt_value(arg, out);
                out.push_str(")\n");
            }
            _ => {
                // Indirect call: cast i64 to function pointer type.
                let fptr = alloc_tmp(tmp);
                out.push_str(&format!("  {} = inttoptr i64 ", fptr));
                fmt_value(func, out);
                out.push_str(" to i64 (i64)*\n");
                out.push_str(&format!("  %{} = call i64 {}(", dest, fptr));
                fmt_value(arg, out);
                out.push_str(")\n");
            }
        },

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

        // ── Tuple ──────────────────────────────────────────────────────────
        SsaOp::Tuple { items } => {
            let count = items.len() as u64;
            out.push_str(&format!(
                "  %{}.tup = call i64 @zutai.tuple_new(i64 {})\n",
                dest, count
            ));
            for (idx, item) in items.iter().enumerate() {
                let value = match item {
                    SsaTupleItem::Named { name: _, value }
                    | SsaTupleItem::Positional(value) => value,
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
        SsaOp::Builtin { op, lhs, rhs } => {
            if builtin_is_cmp(op) {
                // Comparisons yield i1; zext to i64.
                let cmp_tmp = alloc_tmp(tmp);
                out.push_str(&format!("  {} = {} i64 ", cmp_tmp, builtin_ir_op(op)));
                fmt_value(lhs, out);
                out.push_str(", ");
                fmt_value(rhs, out);
                out.push('\n');
                out.push_str(&format!(
                    "  %{} = zext i1 {} to i64\n",
                    dest, cmp_tmp
                ));
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
            // Non-zero value → value, zero → fallback.
            let cmp_tmp = alloc_tmp(tmp);
            out.push_str(&format!("  {} = icmp ne i64 ", cmp_tmp));
            fmt_value(value, out);
            out.push_str(", 0\n");
            out.push_str(&format!("  %{} = select i1 {}, i64 ", dest, cmp_tmp));
            fmt_value(value, out);
            out.push_str(", ");
            fmt_value(fallback, out);
            out.push('\n');
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
            out.push_str(&format!(
                "  %{} = call i64 @zutai.variant_tag(i64 ",
                dest
            ));
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

fn emit_main(out: &mut String, entry_name: &str) {
    let entry = mangle(entry_name);
    out.push_str(&format!(
        "define i32 @main() {{\n  %result = call i64 @{}()\n  call void @zutai.print_i64(i64 %result)\n  ret i32 0\n}}\n",
        entry
    ));
}