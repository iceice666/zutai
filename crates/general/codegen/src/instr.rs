use crate::descriptors::{descriptor_name, llvm_string_bytes};
use crate::preamble::{posit_op_is_cmp, posit_op_name};
use zutai_ssa::*;

use crate::format::*;

// ── Function definitions ───────────────────────────────────────────────────────

pub(crate) fn emit_func_def(out: &mut String, func: &SsaFunc) {
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

pub(crate) fn emit_posit_instr(
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

pub(crate) fn emit_instr(out: &mut String, instr: &SsaInstr, tmp: &mut u64) {
    let dest = mangle(&instr.dest);

    match &instr.op {
        // ── ApplyClosure (D-0003 uniform closure application) ───────────────
        SsaOp::ApplyClosure { closure, arg, tail } => {
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
            let call = if *tail { "musttail call" } else { "call" };
            out.push_str(&format!(
                "  %{} = {} i64 {}(i64 {}, i64 {})\n",
                dest, call, fnptr, closure, arg
            ));
        }

        // ── CallKnown (direct multi-arg call to a known worker) ──────────────
        SsaOp::CallKnown { func, args, tail } => {
            let operands: Vec<String> = args
                .iter()
                .map(|a| format!("i64 {}", emit_value_operand(out, tmp, a)))
                .collect();
            let call = if *tail { "musttail call" } else { "call" };
            out.push_str(&format!(
                "  %{} = {} i64 @{}({})\n",
                dest,
                call,
                mangle(func),
                operands.join(", ")
            ));
        }

        // ── HostPrint (runtime io.print driver) ─────────────────────────────
        SsaOp::HostPrint { value } => {
            let value = emit_value_operand(out, tmp, value);
            out.push_str(&format!("  call void @zutai.print_text(i64 {})\n", value));
            let newline_ptr = emit_symbol_ptr_to_i64(out, tmp, "zutai.main.newline");
            let newline = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {newline} = call i64 @zutai.text_from_global(i64 {}, i64 1)\n",
                newline_ptr
            ));
            out.push_str(&format!("  call void @zutai.print_text(i64 {newline})\n"));
            out.push_str(&format!("  %{} = add i64 {}, 0\n", dest, value));
        }

        // ── HostOp (runtime capability driver) ──────────────────────────────
        SsaOp::HostOp { op, value } => {
            let value = emit_value_operand(out, tmp, value);
            let helper = match op {
                HostOp::IoPrint => "@zutai.host.io_print",
                HostOp::FsRead => "@zutai.host.fs_read",
                HostOp::FsWrite => "@zutai.host.fs_write",
                HostOp::FsOpenRead => "@zutai.host.fs_open_read",
                HostOp::FsReadLine => "@zutai.host.fs_read_line",
                HostOp::FsCloseRead => "@zutai.host.fs_close_read",
                HostOp::FsOpenWrite => "@zutai.host.fs_open_write",
                HostOp::FsWriteText => "@zutai.host.fs_write_text",
                HostOp::FsFlush => "@zutai.host.fs_flush",
                HostOp::FsCloseWrite => "@zutai.host.fs_close_write",
                HostOp::EnvGet => "@zutai.host.env_get",
                HostOp::ClockNow => "@zutai.host.clock_now",
                HostOp::RngNext => "@zutai.host.rng_next",
                HostOp::LoadZti => "@zutai.host.load_zti",
                HostOp::LoadZt => "@zutai.host.load_zt",
                HostOp::NetListen => "@zutai.host.net_listen",
                HostOp::NetAccept => "@zutai.host.net_accept",
                HostOp::NetRead => "@zutai.host.net_read",
                HostOp::NetWrite => "@zutai.host.net_write",
                HostOp::NetClose => "@zutai.host.net_close",
            };
            out.push_str(&format!("  %{} = call i64 {helper}(i64 {})\n", dest, value));
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
        SsaOp::ValueEq {
            negated,
            lhs,
            rhs,
            ty,
        } => {
            let lhs = emit_value_operand(out, tmp, lhs);
            let rhs = emit_value_operand(out, tmp, rhs);
            let desc = emit_symbol_ptr_to_i64(out, tmp, &descriptor_name(*ty));
            let eq = alloc_tmp(tmp);
            out.push_str(&format!(
                "  {eq} = call i64 @zutai.value_eq(i64 {lhs}, i64 {rhs}, i64 {desc})\n"
            ));
            if *negated {
                out.push_str(&format!("  %{dest} = xor i64 {eq}, 1\n"));
            } else {
                out.push_str(&format!("  %{dest} = add i64 {eq}, 0\n"));
            }
        }

        // ── List bridge primitives ──────────────────────────────────────────
        SsaOp::ListPrim { op, args } => {
            let operands: Vec<String> = args
                .iter()
                .map(|a| emit_value_operand(out, tmp, a))
                .collect();
            let (callee, operands) = match op {
                DfListPrimOp::Cons => ("zutai.list_cons", operands.as_slice()),
                DfListPrimOp::Append => ("zutai.list_append", operands.as_slice()),
                DfListPrimOp::IsNil => ("zutai.list_is_nil", operands.as_slice()),
                DfListPrimOp::Head => ("zutai.list_head", operands.as_slice()),
                DfListPrimOp::Tail => ("zutai.list_tail", operands.as_slice()),
                DfListPrimOp::FoldlStrict => ("zutai.list_foldl_strict", operands.as_slice()),
            };
            let arglist = operands
                .iter()
                .map(|v| format!("i64 {}", v))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "  %{} = call i64 @{}({})\n",
                dest, callee, arglist
            ));
        }

        // ── Numeric bridge primitives ───────────────────────────────────────
        SsaOp::NumPrim { op, args } => {
            let operands: Vec<String> = args
                .iter()
                .map(|a| emit_value_operand(out, tmp, a))
                .collect();
            let callee = match op {
                DfNumPrimOp::Abs => "zutai.num_abs",
                DfNumPrimOp::Rem => "zutai.num_rem",
                DfNumPrimOp::Pow => "zutai.num_pow",
                DfNumPrimOp::ToFloat => "zutai.num_to_float",
                DfNumPrimOp::Round => "zutai.num_round",
                DfNumPrimOp::Truncate => "zutai.num_truncate",
                DfNumPrimOp::FloatAdd => "zutai.float_add",
                DfNumPrimOp::FloatSub => "zutai.float_sub",
                DfNumPrimOp::FloatMul => "zutai.float_mul",
                DfNumPrimOp::FloatDiv => "zutai.float_div",
                DfNumPrimOp::FloatLt => "zutai.float_lt",
                DfNumPrimOp::FloatLe => "zutai.float_le",
                DfNumPrimOp::FloatGt => "zutai.float_gt",
                DfNumPrimOp::FloatGe => "zutai.float_ge",
            };
            let arglist = operands
                .iter()
                .map(|v| format!("i64 {}", v))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "  %{} = call i64 @{}({})\n",
                dest, callee, arglist
            ));
        }

        // ── Text bridge primitives ──────────────────────────────────────────
        SsaOp::TextPrim { op, args } => {
            let operands: Vec<String> = args
                .iter()
                .map(|a| emit_value_operand(out, tmp, a))
                .collect();
            let callee = match op {
                DfTextPrimOp::Eq => "zutai.text_eq",
                DfTextPrimOp::Ne => "zutai.text_ne",
                DfTextPrimOp::Lt => "zutai.text_lt",
                DfTextPrimOp::Le => "zutai.text_le",
                DfTextPrimOp::Gt => "zutai.text_gt",
                DfTextPrimOp::Ge => "zutai.text_ge",
                DfTextPrimOp::Length => "zutai.text_length",
                DfTextPrimOp::Split => "zutai.text_split",
                DfTextPrimOp::Join => "zutai.text_join",
                DfTextPrimOp::Trim => "zutai.text_trim",
                DfTextPrimOp::ToUpper => "zutai.text_to_upper",
                DfTextPrimOp::ToLower => "zutai.text_to_lower",
                DfTextPrimOp::Contains => "zutai.text_contains",
                DfTextPrimOp::Replace => "zutai.text_replace",
                DfTextPrimOp::Show => "zutai.text_show",
                DfTextPrimOp::ParseInt => "zutai.text_parse_int",
                DfTextPrimOp::ParseFloat => "zutai.text_parse_float",
            };
            let arglist = operands
                .iter()
                .map(|v| format!("i64 {}", v))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "  %{} = call i64 @{}({})\n",
                dest, callee, arglist
            ));
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

pub(crate) fn emit_terminator(out: &mut String, term: &SsaTerminator, tmp: &mut u64) {
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

pub(crate) fn alloc_tmp(tmp: &mut u64) -> String {
    let id = *tmp;
    *tmp += 1;
    format!("%_tmp.{}", id)
}

// ── @main ─────────────────────────────────────────────────────────────────────

pub(crate) fn emit_main(out: &mut String, entry_name: &str, entry_desc: &str) {
    let entry = mangle(entry_name);
    let newline = llvm_string_bytes("\n");
    out.push_str(&format!(
        "@zutai.main.newline = private unnamed_addr constant [{} x i8] c\"{}\"\n\n",
        newline.len, newline.escaped
    ));
    out.push_str("define i32 @main() {\n");
    let mut tmp = 0u64;
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

// ── Native library exports ───────────────────────────────────────────────────

pub(crate) fn emit_library_exports(out: &mut String, entry_name: &str, entry_desc: &str) {
    let entry = mangle(entry_name);

    out.push_str("define i64 @zutai_entry() {\n");
    out.push_str(&format!("  %result = call i64 @{}()\n", entry));
    out.push_str("  ret i64 %result\n}\n\n");

    out.push_str("define i64 @zutai_entry_descriptor() {\n");
    let mut tmp = 0u64;
    let desc = emit_symbol_ptr_to_i64(out, &mut tmp, entry_desc);
    out.push_str(&format!("  ret i64 {}\n", desc));
    out.push_str("}\n\n");

    out.push_str("define i64 @zutai_entry_json() {\n");
    let mut tmp = 0u64;
    out.push_str(&format!("  %result = call i64 @{}()\n", entry));
    let desc = emit_symbol_ptr_to_i64(out, &mut tmp, entry_desc);
    out.push_str(&format!(
        "  %json = call i64 @zutai.to_json(i64 %result, i64 {})\n",
        desc
    ));
    out.push_str("  ret i64 %json\n}\n");
}
