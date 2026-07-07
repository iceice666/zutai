use crate::descriptors::{collect_from_func, llvm_string_bytes};
use zutai_ssa::*;

use crate::collect_functions;
use crate::format::*;

// ── Preamble & declarations ────────────────────────────────────────────────────

pub(crate) fn host_target_triple() -> &'static str {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        _ => "x86_64-unknown-linux-gnu",
    }
}

pub(crate) fn host_target_datalayout() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => {
            Some("e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128")
        }
        _ => None,
    }
}

pub(crate) fn emit_preamble(out: &mut String) {
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

pub(crate) fn emit_type_decls(out: &mut String) {
    out.push_str("; Zutai runtime types (v0: all values are i64)\n\n");
}

pub(crate) fn emit_runtime_decls(out: &mut String) {
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
    out.push_str("declare i64 @zutai.to_json(i64, i64)\n");

    // Host capability operations
    out.push_str("declare i64 @zutai.host.io_print(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_read(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_write(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_open_read(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_read_line(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_close_read(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_open_write(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_write_text(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_flush(i64)\n");
    out.push_str("declare i64 @zutai.host.fs_close_write(i64)\n");
    out.push_str("declare i64 @zutai.host.env_get(i64)\n");
    out.push_str("declare i64 @zutai.host.clock_now(i64)\n");
    out.push_str("declare i64 @zutai.host.rng_next(i64)\n");
    out.push_str("declare i64 @zutai.host.load_zti(i64)\n");
    out.push_str("declare i64 @zutai.host.load_zt(i64)\n");

    // TCP / network capability operations
    out.push_str("declare i64 @zutai.host.net_listen(i64)\n");
    out.push_str("declare i64 @zutai.host.net_accept(i64)\n");
    out.push_str("declare i64 @zutai.host.net_read(i64)\n");
    out.push_str("declare i64 @zutai.host.net_write(i64)\n");
    out.push_str("declare i64 @zutai.host.net_close(i64)\n");

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
    out.push_str("declare i64 @zutai.list_is_nil(i64)\n");
    out.push_str("declare i64 @zutai.list_head(i64)\n");
    out.push_str("declare i64 @zutai.list_tail(i64)\n");
    out.push_str("declare i64 @zutai.list_foldl_strict(i64, i64, i64)\n");

    // Numeric bridge operations
    out.push_str("declare i64 @zutai.num_abs(i64)\n");
    out.push_str("declare i64 @zutai.num_rem(i64, i64)\n");
    out.push_str("declare i64 @zutai.num_pow(i64, i64)\n");
    out.push_str("declare i64 @zutai.num_to_float(i64)\n");
    out.push_str("declare i64 @zutai.num_round(i64)\n");
    out.push_str("declare i64 @zutai.num_truncate(i64)\n");
    out.push_str("declare i64 @zutai.float_add(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_sub(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_mul(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_div(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_lt(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_le(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_gt(i64, i64)\n");
    out.push_str("declare i64 @zutai.float_ge(i64, i64)\n");
    // Optional/Maybe operations
    out.push_str("declare i64 @zutai.coalesce(i64, i64)\n");
    // Type-directed value operations
    out.push_str("declare i64 @zutai.value_eq(i64, i64, i64)\n");
    // Variant operations
    out.push_str("declare i64 @zutai.variant_new(i64, i64)\n");
    out.push_str("declare i64 @zutai.variant_tag(i64)\n");
    out.push_str("declare i64 @zutai.variant_value(i64)\n");

    // Text operations
    out.push_str("declare i64 @zutai.text_from_global(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_concat(i64, i64)\n");
    out.push_str("declare i64 @zutai.atom_from_global(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_eq(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_ne(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_lt(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_le(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_gt(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_ge(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_length(i64)\n");
    out.push_str("declare i64 @zutai.text_split(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_join(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_trim(i64)\n");
    out.push_str("declare i64 @zutai.text_to_upper(i64)\n");
    out.push_str("declare i64 @zutai.text_to_lower(i64)\n");
    out.push_str("declare i64 @zutai.text_contains(i64, i64)\n");
    out.push_str("declare i64 @zutai.text_replace(i64, i64, i64)\n");
    out.push_str("declare i64 @zutai.text_show(i64)\n");
    out.push_str("declare i64 @zutai.text_parse_int(i64)\n");
    out.push_str("declare i64 @zutai.text_parse_float(i64)\n");

    // C stdlib
    out.push_str("declare i64 @exit(i64)\n\n");
}

pub(crate) fn emit_posit_runtime_decls(module: &SsaModule, out: &mut String) {
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

pub(crate) fn posit_op_name(op: DfPositOp) -> &'static str {
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

pub(crate) fn posit_op_is_cmp(op: DfPositOp) -> bool {
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
pub(crate) fn emit_static_closures(out: &mut String, module: &SsaModule) {
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
pub(crate) enum Constant {
    Text(String),
    Atom(String),
}

pub(crate) fn emit_static_text(out: &mut String, prefix: &str, s: &str) {
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

pub(crate) fn collect_and_emit_constants(module: &SsaModule, out: &mut String) {
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
