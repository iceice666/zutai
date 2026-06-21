//! LLVM IR text emission from Zutai SSA modules.
//!
//! Produces `.ll` files for the host LLVM target.
//! All Zutai values are represented as `i64` for v0 simplicity:
//! integers are stored directly, booleans as 0/1, and compound
//! values (records, tuples, lists, closures, text) are heap-allocated
//! with their pointer cast to `i64`.

use std::collections::HashMap;

use zutai_ssa::*;

// ── Public API ─────────────────────────────────────────────────────────────────

/// Emit a complete LLVM IR `.ll` file from an SSA module.
pub fn emit_llvm(module: &SsaModule) -> String {
    emit_llvm_with_host_prints(module, &[])
}

/// Emit LLVM IR and replay already-elaborated host `io.print` text at `@main`.
pub fn emit_llvm_with_host_prints(module: &SsaModule, host_prints: &[String]) -> String {
    let mut out = String::with_capacity(8192);
    emit_preamble(&mut out);
    emit_type_decls(&mut out);
    emit_runtime_decls(&mut out);
    emit_posit_runtime_decls(module, &mut out);
    collect_and_emit_constants(module, &mut out);
    let entry_desc = emit_descriptors(module, &mut out);
    emit_static_closures(&mut out, module);

    let all_funcs = collect_functions(module);
    for func in &all_funcs {
        emit_func_def(&mut out, func);
    }

    emit_main(&mut out, &module.entry.name, &entry_desc, host_prints);
    out
}

/// Return why the module's entry value cannot be rendered by the native ABI.
pub fn unsupported_entry_type_reason(module: &SsaModule) -> Option<&'static str> {
    match &module.entry_ty {
        DfTy::Fun(_, _) => Some(
            "compiled entry point returns a function, which cannot be shown by the v0 runtime ABI",
        ),
        DfTy::Type => {
            Some("compiled entry point returns Type, which cannot be shown by the v0 runtime ABI")
        }
        _ => None,
    }
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

const TAG_TEXT: u64 = 6;
const DESC_INT: i64 = 0;
const DESC_BOOL: i64 = 1;
const DESC_FLOAT: i64 = 2;
const DESC_TEXT: i64 = 3;
const DESC_ATOM: i64 = 4;
const DESC_RECORD: i64 = 5;
const DESC_TUPLE: i64 = 6;
const DESC_LIST: i64 = 7;
const DESC_OPTIONAL: i64 = 8;
const DESC_MAYBE: i64 = 9;
const DESC_VARIANT: i64 = 10;
const DESC_POSIT: i64 = 11;

fn object_header(tag: u64, count: usize) -> u64 {
    ((count as u64) << 8) | tag
}

/// Global symbol name of the static closure object for a top-level function.
fn closure_global_name(name: &str) -> String {
    format!("zutai.closure.{}", mangle(name))
}

enum StaticWord {
    I64(String),
    Ptr(String),
}

fn i64_word(value: impl std::fmt::Display) -> StaticWord {
    StaticWord::I64(value.to_string())
}

fn ptr_word(symbol: impl Into<String>) -> StaticWord {
    StaticWord::Ptr(symbol.into())
}

fn emit_static_words(out: &mut String, name: &str, linkage: &str, words: &[StaticWord]) {
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
fn fmt_value(val: &SsaValue) -> String {
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
fn fmt_lit(lit: &DfLit) -> String {
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
            // Encode float as its IEEE 754 double bit pattern in an i64.
            // This lets us store the exact float value in our uniform i64 type.
            format!("0x{:016x}", f.to_bits())
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

fn emit_symbol_ptr_to_i64(out: &mut String, tmp: &mut u64, symbol: &str) -> String {
    let name = alloc_tmp(tmp);
    out.push_str(&format!("  {name} = ptrtoint ptr @{symbol} to i64\n"));
    name
}

fn emit_value_operand(out: &mut String, tmp: &mut u64, value: &SsaValue) -> String {
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

fn fmt_phi_value(value: &SsaValue) -> String {
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
fn str_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
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

fn host_target_triple() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        _ => "x86_64-unknown-linux-gnu",
    }
}

fn host_target_datalayout() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => {
            Some("e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128")
        }
        _ => None,
    }
}

fn emit_preamble(out: &mut String) {
    out.push_str("target triple = \"");
    out.push_str(host_target_triple());
    out.push_str("\"\n");
    if let Some(layout) = host_target_datalayout() {
        out.push_str("target datalayout = \"");
        out.push_str(layout);
        out.push_str("\"\n");
    }
    out.push('\n');
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
    out.push_str("declare void @zutai.show(i64, i64)\n");

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
    out.push_str("declare i64 @zutai.atom_from_global(i64, i64)\n");

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

/// Emit the static empty-capture closure object for every top-level function so
/// that `GlobalClosure(name)` resolves to `@zutai.closure.<name>` (D-0003).
fn emit_static_closures(out: &mut String, module: &SsaModule) {
    if module.closure_exports.is_empty() {
        return;
    }
    out.push_str("; ── Static closures ───────────────────────────────────────────\n\n");
    for name in &module.closure_exports {
        let words = [i64_word(closure_header(0)), ptr_word(mangle(name))];
        emit_static_words(out, &closure_global_name(name), "internal", &words);
    }
    out.push('\n');
}

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
enum Constant {
    Text(String),
    Atom(String),
}

fn emit_static_text(out: &mut String, prefix: &str, s: &str) {
    let hash = str_hash(s);
    let esc = llvm_string_bytes(s);
    let data = format!("zutai.{prefix}.data.{hash}");
    let obj = format!("zutai.{prefix}.{hash}");
    out.push_str(&format!(
        "@{} = private unnamed_addr constant [{} x i8] c\"{}\"\n",
        data, esc.len, esc.escaped
    ));
    let words = [
        i64_word(object_header(TAG_TEXT, 0)),
        i64_word(s.len()),
        ptr_word(data),
    ];
    emit_static_words(out, &obj, "private", &words);
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
    constants.sort();
    constants.dedup();
    if constants.is_empty() {
        return;
    }
    out.push_str("; ── Global constants ───────────────────────────────────────────\n\n");
    for c in &constants {
        match c {
            Constant::Text(s) => emit_static_text(out, "text", s),
            Constant::Atom(s) => emit_static_text(out, "atom", s),
        }
    }
    out.push('\n');
}

// ── Type descriptors ───────────────────────────────────────────────────────────

fn desc_ref(name: &str) -> StaticWord {
    ptr_word(name)
}

fn emit_descriptors(module: &SsaModule, out: &mut String) -> String {
    out.push_str("; ── Type descriptors ─────────────────────────────────────────\n\n");
    let mut emitter = DescriptorEmitter {
        types: &module.types,
        out,
        cache: HashMap::new(),
        strings: HashMap::new(),
        next: 0,
    };
    let entry = emitter.emit(module.entry_ty_id);
    emitter.out.push('\n');
    entry
}

struct DescriptorEmitter<'a, 'o> {
    types: &'a DfTypes,
    out: &'o mut String,
    cache: HashMap<DfTyId, String>,
    strings: HashMap<String, String>,
    next: usize,
}

impl<'a, 'o> DescriptorEmitter<'a, 'o> {
    fn emit(&mut self, ty_id: DfTyId) -> String {
        if let Some(name) = self.cache.get(&ty_id) {
            return name.clone();
        }
        let name = format!("zutai.desc.{}", self.next);
        self.next += 1;
        self.cache.insert(ty_id, name.clone());

        let ty = self.types[ty_id].clone();
        let words = self.words_for_ty(ty);
        emit_static_words(self.out, &name, "private", &words);
        name
    }

    fn words_for_ty(&mut self, ty: DfTy) -> Vec<StaticWord> {
        match ty {
            DfTy::Int => vec![i64_word(DESC_INT)],
            DfTy::Bool | DfTy::True | DfTy::False => vec![i64_word(DESC_BOOL)],
            DfTy::Float => vec![i64_word(DESC_FLOAT)],
            DfTy::Text => vec![i64_word(DESC_TEXT)],
            DfTy::Atom => vec![i64_word(DESC_ATOM)],
            DfTy::Posit(spec) => vec![
                i64_word(DESC_POSIT),
                i64_word(spec.nbits),
                i64_word(spec.es),
            ],
            DfTy::List(inner) => vec![i64_word(DESC_LIST), desc_ref(&self.emit(inner))],
            DfTy::Optional(inner) => vec![i64_word(DESC_OPTIONAL), desc_ref(&self.emit(inner))],
            DfTy::Maybe(inner) => vec![i64_word(DESC_MAYBE), desc_ref(&self.emit(inner))],
            DfTy::Record(fields) => {
                let mut words = Vec::with_capacity(2 + fields.len() * 3);
                words.push(i64_word(DESC_RECORD));
                words.push(i64_word(fields.len()));
                for field in fields {
                    let (ptr, len) = self.string_ref(&field.name);
                    words.push(ptr);
                    words.push(i64_word(len));
                    words.push(desc_ref(&self.emit(field.ty)));
                }
                words
            }
            DfTy::Tuple(fields) => {
                let mut words = Vec::with_capacity(2 + fields.len() * 4);
                words.push(i64_word(DESC_TUPLE));
                words.push(i64_word(fields.len()));
                for field in fields {
                    match field {
                        DfTupleField::Named { name, ty } => {
                            let (ptr, len) = self.string_ref(&name);
                            words.push(i64_word(1));
                            words.push(ptr);
                            words.push(i64_word(len));
                            words.push(desc_ref(&self.emit(ty)));
                        }
                        DfTupleField::Positional(ty) => {
                            words.push(i64_word(0));
                            words.push(i64_word(0));
                            words.push(i64_word(0));
                            words.push(desc_ref(&self.emit(ty)));
                        }
                    }
                }
                words
            }
            DfTy::Union(members) => {
                let mut words = Vec::with_capacity(2 + members.len() * 3);
                words.push(i64_word(DESC_VARIANT));
                words.push(i64_word(members.len()));
                for member in members {
                    let (ptr, len) = self.string_ref(&member.tag);
                    words.push(ptr);
                    words.push(i64_word(len));
                    words.push(desc_ref(&self.emit(member.ty)));
                }
                words
            }
            DfTy::Fun(_, _)
            | DfTy::TyVar(_)
            | DfTy::TyFun(_, _)
            | DfTy::TyApp(_, _)
            | DfTy::Type
            | DfTy::Error => {
                vec![i64_word(DESC_INT)]
            }
        }
    }

    fn string_ref(&mut self, s: &str) -> (StaticWord, usize) {
        if let Some(name) = self.strings.get(s) {
            return (ptr_word(name.clone()), s.len());
        }
        let hash = str_hash(s);
        let name = format!("zutai.desc.str.{hash}");
        let esc = llvm_string_bytes(s);
        self.out.push_str(&format!(
            "@{} = private unnamed_addr constant [{} x i8] c\"{}\"\n",
            name, esc.len, esc.escaped
        ));
        self.strings.insert(s.to_string(), name.clone());
        (ptr_word(name), s.len())
    }
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
        SsaOp::CallGlobal { .. } => {}
        SsaOp::TyApp { .. } => {}
        SsaOp::Record { fields } => {
            for v in fields {
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
        SsaOp::Select { base, slot: _ } => collect_from_value(base, constants),
        SsaOp::Variant { value, .. } => collect_from_value(value, constants),
        SsaOp::VariantValue { scrutinee } => collect_from_value(scrutinee, constants),
        SsaOp::Builtin { op: _, lhs, rhs } => {
            collect_from_value(lhs, constants);
            collect_from_value(rhs, constants);
        }
        SsaOp::Coalesce { value, fallback } => {
            collect_from_value(value, constants);
            collect_from_value(fallback, constants);
        }
        SsaOp::Error => {}
        SsaOp::Alias { value } => collect_from_value(value, constants),
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
    let lhs = emit_value_operand(out, tmp, lhs);
    let rhs = emit_value_operand(out, tmp, rhs);
    let (nbits, es) = spec;
    let helper = format!("@zutai.posit{nbits}e{es}.{}", posit_op_name(op));
    match (nbits, posit_op_is_cmp(op)) {
        (32, false) => {
            let lhs32 = alloc_tmp(tmp);
            let rhs32 = alloc_tmp(tmp);
            let call = alloc_tmp(tmp);
            out.push_str(&format!("  {lhs32} = trunc i64 {lhs} to i32\n"));
            out.push_str(&format!("  {rhs32} = trunc i64 {rhs} to i32\n"));
            out.push_str(&format!(
                "  {call} = call i32 {helper}(i32 {lhs32}, i32 {rhs32})\n"
            ));
            out.push_str(&format!("  %{dest} = zext i32 {call} to i64\n"));
        }
        (32, true) => {
            let lhs32 = alloc_tmp(tmp);
            let rhs32 = alloc_tmp(tmp);
            let call = alloc_tmp(tmp);
            out.push_str(&format!("  {lhs32} = trunc i64 {lhs} to i32\n"));
            out.push_str(&format!("  {rhs32} = trunc i64 {rhs} to i32\n"));
            out.push_str(&format!(
                "  {call} = call i1 {helper}(i32 {lhs32}, i32 {rhs32})\n"
            ));
            out.push_str(&format!("  %{dest} = zext i1 {call} to i64\n"));
        }
        (64, false) => {
            out.push_str(&format!(
                "  %{dest} = call i64 {helper}(i64 {lhs}, i64 {rhs})\n"
            ));
        }
        (64, true) => {
            let call = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {call} = call i1 {helper}(i64 {lhs}, i64 {rhs})\n"
            ));
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
            let closure = emit_value_operand(out, tmp, closure);
            let cptr = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", cptr, closure));
            let code_slot = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {} = getelementptr i64, ptr {}, i64 1\n",
                code_slot, cptr
            ));
            let code = alloc_tmp(tmp);
            out.push_str(&format!("  {} = load i64, ptr {}\n", code, code_slot));
            let fnptr = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", fnptr, code));
            let arg = emit_value_operand(out, tmp, arg);
            out.push_str(&format!(
                "  %{} = call i64 {}(i64 {}, i64 {})\n",
                dest, fnptr, closure, arg
            ));
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
            let code_ptr = emit_symbol_ptr_to_i64(out, tmp, &mangle(code));
            out.push_str(&format!("  store i64 {}, ptr {}\n", code_ptr, code_slot));
            for (index, cap) in captures.iter().enumerate() {
                let slot = alloc_tmp(tmp);
                out.push_str(&format!(
                    "  {} = getelementptr i64, ptr {}, i64 {}\n",
                    slot,
                    base,
                    2 + index
                ));
                let cap = emit_value_operand(out, tmp, cap);
                out.push_str(&format!("  store i64 {}, ptr {}\n", cap, slot));
            }
            out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, raw));
        }

        // ── LoadCapture (read a capture from the enclosing closure) ─────────
        SsaOp::LoadCapture { closure, index } => {
            let closure = emit_value_operand(out, tmp, closure);
            let cptr = alloc_tmp(tmp);
            out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", cptr, closure));
            let slot = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {} = getelementptr i64, ptr {}, i64 {}\n",
                slot,
                cptr,
                2 + index
            ));
            out.push_str(&format!("  %{} = load i64, ptr {}\n", dest, slot));
        }

        // ── CallGlobal (force a top-level thunk) ────────────────────────────
        SsaOp::CallGlobal { name } => {
            out.push_str(&format!("  %{} = call i64 @{}()\n", dest, mangle(name)));
        }

        // ── TyApp (erased) ─────────────────────────────────────────────────
        SsaOp::TyApp { poly, ty_args: _ } => {
            // Type application is erased at runtime; copy the value.
            let poly = emit_value_operand(out, tmp, poly);
            out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, poly));
        }

        // ── Record ─────────────────────────────────────────────────────────
        SsaOp::Record { fields } => {
            let count = fields.len() as u64;
            out.push_str(&format!(
                "  %{}.rec = call i64 @zutai.record_new(i64 {})\n",
                dest, count
            ));
            for (idx, value) in fields.iter().enumerate() {
                let value = emit_value_operand(out, tmp, value);
                out.push_str(&format!(
                    "  call void @zutai.record_set(i64 %{}.rec, i64 {}, i64 {})\n",
                    dest, idx, value
                ));
            }
            out.push_str(&format!("  %{} = add i64 %{}.rec, 0\n", dest, dest));
        }

        // ── Record update ───────────────────────────────────────────────────
        SsaOp::RecordUpdate { base, updates } => {
            let base = emit_value_operand(out, tmp, base);
            if updates.is_empty() {
                out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, base));
            } else {
                let mut prev = base;
                for (idx, (slot, value)) in updates.iter().enumerate() {
                    let value = emit_value_operand(out, tmp, value);
                    let tmp_name = format!("%{}.upd{}", dest, idx);
                    out.push_str(&format!(
                        "  {} = call i64 @zutai.record_update(i64 {}, i64 {}, i64 {})\n",
                        tmp_name, prev, slot, value
                    ));
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
                let value = emit_value_operand(out, tmp, value);
                out.push_str(&format!(
                    "  call void @zutai.tuple_set(i64 %{}.tup, i64 {}, i64 {})\n",
                    dest, idx, value
                ));
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
                    let elem = emit_value_operand(out, tmp, elem);
                    let cons_tmp = if i == 0 {
                        format!("%{}", dest)
                    } else {
                        alloc_tmp(tmp)
                    };
                    out.push_str(&format!(
                        "  {} = call i64 @zutai.list_cons(i64 {}, i64 {})\n",
                        cons_tmp, elem, prev
                    ));
                    prev = cons_tmp;
                }
            }
        }

        // ── Select ──────────────────────────────────────────────────────────
        SsaOp::Select { base, slot } => {
            let base = emit_value_operand(out, tmp, base);
            out.push_str(&format!(
                "  %{} = call i64 @zutai.record_get(i64 {}, i64 {})\n",
                dest, base, slot
            ));
        }

        // ── Variant ─────────────────────────────────────────────────────────
        SsaOp::Variant {
            tag_index, value, ..
        } => {
            let value = emit_value_operand(out, tmp, value);
            out.push_str(&format!(
                "  %{} = call i64 @zutai.variant_new(i64 {}, i64 {})\n",
                dest, tag_index, value
            ));
        }

        // ── Variant payload ─────────────────────────────────────────────────
        SsaOp::VariantValue { scrutinee } => {
            let scrutinee = emit_value_operand(out, tmp, scrutinee);
            out.push_str(&format!(
                "  %{} = call i64 @zutai.variant_value(i64 {})\n",
                dest, scrutinee
            ));
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
            let lhs = emit_value_operand(out, tmp, lhs);
            let rhs = emit_value_operand(out, tmp, rhs);
            if builtin_is_cmp(op) {
                // Comparisons yield i1; zext to i64.
                let cmp_tmp = alloc_tmp(tmp);
                out.push_str(&format!(
                    "  {} = {} i64 {}, {}\n",
                    cmp_tmp,
                    builtin_ir_op(op),
                    lhs,
                    rhs
                ));
                out.push_str(&format!("  %{} = zext i1 {} to i64\n", dest, cmp_tmp));
            } else {
                // Arithmetic / bitwise on i64.
                out.push_str(&format!(
                    "  %{} = {} i64 {}, {}\n",
                    dest,
                    builtin_ir_op(op),
                    lhs,
                    rhs
                ));
            }
        }

        // ── Coalesce ────────────────────────────────────────────────────────
        SsaOp::Coalesce { value, fallback } => {
            // @zutai.coalesce unwraps one Optional or Maybe layer:
            // #none/#absent choose fallback; #some (x)/#present (x) return x.
            let value = emit_value_operand(out, tmp, value);
            let fallback = emit_value_operand(out, tmp, fallback);
            out.push_str(&format!(
                "  %{} = call i64 @zutai.coalesce(i64 {}, i64 {})\n",
                dest, value, fallback
            ));
        }

        // ── Error ───────────────────────────────────────────────────────────
        SsaOp::Error => {
            out.push_str(&format!("  %{} = add i64 0, 0\n", dest));
        }

        // ── Alias ───────────────────────────────────────────────────────────
        SsaOp::Alias { value } => {
            let value = emit_value_operand(out, tmp, value);
            out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, value));
        }

        // ── Phi ─────────────────────────────────────────────────────────────
        SsaOp::Phi { branches } => {
            out.push_str(&format!("  %{} = phi i64 ", dest));
            for (i, (label, val)) in branches.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push('[');
                out.push_str(&fmt_phi_value(val));
                out.push_str(&format!(", %{}]", mangle(label)));
            }
            out.push('\n');
        }

        // ── MatchDiscriminant ───────────────────────────────────────────────
        SsaOp::MatchDiscriminant { scrutinee } => {
            let scrutinee = emit_value_operand(out, tmp, scrutinee);
            out.push_str(&format!(
                "  %{} = call i64 @zutai.variant_tag(i64 {})\n",
                dest, scrutinee
            ));
        }
    }
}

fn emit_terminator(out: &mut String, term: &SsaTerminator, tmp: &mut u64) {
    match term {
        SsaTerminator::Return(val) => {
            let val = emit_value_operand(out, tmp, val);
            out.push_str(&format!("  ret i64 {}\n", val));
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
            let cond = emit_value_operand(out, tmp, cond);
            let cond_tmp = alloc_tmp(tmp);
            out.push_str(&format!("  {} = icmp ne i64 {}, 0\n", cond_tmp, cond));
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

fn emit_main(out: &mut String, entry_name: &str, entry_desc: &str, host_prints: &[String]) {
    let entry = mangle(entry_name);
    for (index, text) in host_prints.iter().enumerate() {
        let bytes = llvm_string_bytes(text);
        out.push_str(&format!(
            "@zutai.effect.print.{index} = private unnamed_addr constant [{} x i8] c\"{}\"\n",
            bytes.len, bytes.escaped
        ));
    }
    let newline = llvm_string_bytes("\n");
    out.push_str(&format!(
        "@zutai.main.newline = private unnamed_addr constant [{} x i8] c\"{}\"\n\n",
        newline.len, newline.escaped
    ));
    out.push_str("define i32 @main() {\n");
    let mut tmp = 0u64;
    for (index, text) in host_prints.iter().enumerate() {
        let print_symbol = format!("zutai.effect.print.{index}");
        let print_ptr = emit_symbol_ptr_to_i64(out, &mut tmp, &print_symbol);
        out.push_str(&format!(
            "  %effect_print_{index} = call i64 @zutai.text_from_global(i64 {}, i64 {})\n",
            print_ptr,
            text.len()
        ));
        out.push_str(&format!(
            "  call void @zutai.print_text(i64 %effect_print_{index})\n"
        ));
        let newline_ptr = emit_symbol_ptr_to_i64(out, &mut tmp, "zutai.main.newline");
        out.push_str(&format!(
            "  %effect_print_newline_{index} = call i64 @zutai.text_from_global(i64 {}, i64 1)\n",
            newline_ptr
        ));
        out.push_str(&format!(
            "  call void @zutai.print_text(i64 %effect_print_newline_{index})\n"
        ));
    }
    out.push_str(&format!("  %result = call i64 @{}()\n", entry));
    let entry_desc = emit_symbol_ptr_to_i64(out, &mut tmp, entry_desc);
    out.push_str(&format!(
        "  call void @zutai.show(i64 %result, i64 {})\n",
        entry_desc
    ));
    let newline_ptr = emit_symbol_ptr_to_i64(out, &mut tmp, "zutai.main.newline");
    out.push_str(&format!(
        "  %newline = call i64 @zutai.text_from_global(i64 {}, i64 1)\n",
        newline_ptr
    ));
    out.push_str("  call void @zutai.print_text(i64 %newline)\n");
    out.push_str("  ret i32 0\n}\n");
}

#[cfg(test)]
mod tests {
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
            llvm.contains(
                "@zutai.closure.inc = internal constant { i64, ptr } { i64 7, ptr @inc }"
            ),
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
}
