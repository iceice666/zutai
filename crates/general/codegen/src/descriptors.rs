use std::collections::HashMap;

use zutai_ssa::*;

use crate::format::*;
use crate::preamble::Constant;

// ── Type descriptors ───────────────────────────────────────────────────────────

pub(crate) fn desc_ref(name: &str) -> StaticWord {
    ptr_word(name)
}

pub(crate) fn emit_descriptors(module: &SsaModule, out: &mut String) -> String {
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

pub(crate) struct DescriptorEmitter<'a, 'o> {
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

pub(crate) fn collect_from_func(func: &SsaFunc, constants: &mut Vec<Constant>) {
    for block in &func.blocks {
        for instr in &block.instructions {
            collect_from_op(&instr.op, constants);
        }
        collect_from_terminator(&block.terminator, constants);
    }
}

pub(crate) fn collect_from_op(op: &SsaOp, constants: &mut Vec<Constant>) {
    match op {
        SsaOp::ApplyClosure { closure, arg } => {
            collect_from_value(closure, constants);
            collect_from_value(arg, constants);
        }
        SsaOp::HostPrint { value } => collect_from_value(value, constants),
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

pub(crate) fn collect_from_value(val: &SsaValue, constants: &mut Vec<Constant>) {
    match val {
        SsaValue::Lit(DfLit::Text(s)) => constants.push(Constant::Text(s.clone())),
        SsaValue::Lit(DfLit::Atom(s)) => constants.push(Constant::Atom(s.clone())),
        _ => {}
    }
}

pub(crate) fn collect_from_terminator(term: &SsaTerminator, constants: &mut Vec<Constant>) {
    match term {
        SsaTerminator::Return(v) => collect_from_value(v, constants),
        SsaTerminator::Jump(_) => {}
        SsaTerminator::Branch { cond, .. } => collect_from_value(cond, constants),
    }
}

pub(crate) struct EscapedString {
    pub(crate) len: usize,
    pub(crate) escaped: String,
}

/// Escape a Rust string into the LLVM IR `c"..."` byte-literal format,
/// including a null terminator.
pub(crate) fn llvm_string_bytes(s: &str) -> EscapedString {
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
