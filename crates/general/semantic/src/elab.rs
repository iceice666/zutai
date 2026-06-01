//! Elaboration: `HirType → Ty`.
//!
//! `ty_of_hir` converts a `HirTypeId` to an interned `TyId`. `elab_file`
//! walks all declaration type annotations and writes `Symbol::ty` back so
//! M2 can start from populated types rather than `Unknown` everywhere.

use zutai_hir::arena::Arena;
use zutai_hir::decl::HirDecl;
use zutai_hir::file::HirFile;
use zutai_hir::symbol::{SymbolId, SymbolKind, SymbolTable};
use zutai_hir::ty::{FieldKind as HirFieldKind, HirTyRef, HirType, HirTypeId, HirTypeKind, LitVal};

use crate::ty::{
    BOOL_TY, FLOAT_TY, FieldKind, INT_TY, NONE_TY, RecordField, TEXT_TY, Ty, TyId, TyInterner,
    UNKNOWN_TY,
};

/// Walk all declaration type annotations in `hir`, elaborate each to a `TyId`,
/// and write the result back into `Symbol::ty`.
pub fn elab_file(hir: &mut HirFile, interner: &mut TyInterner) {
    let decl_ids = hir.decls.clone();
    let mut annotations: Vec<(SymbolId, TyId)> = Vec::new();

    for decl_id in decl_ids {
        let (sym_id, opt_ty_id) = match hir.decls_arena.get(decl_id) {
            HirDecl::Value { name, ty, .. } => (*name, *ty),
            HirDecl::Function { name, sig, .. } => (*name, *sig),
            HirDecl::TypeDef { name, body, .. } => (*name, Some(*body)),
        };

        if let Some(ty_id) = opt_ty_id {
            let resolved = ty_of_hir(ty_id, &hir.types, &hir.symbols, interner);
            annotations.push((sym_id, resolved));
        }
    }

    for (sym_id, ty_id) in annotations {
        hir.symbols.get_mut(sym_id).ty = Some(HirTyRef(ty_id.0));
    }
}

/// Elaborate a single `HirTypeId` into an interned `TyId`.
pub fn ty_of_hir(
    type_id: HirTypeId,
    types: &Arena<HirType>,
    symbols: &SymbolTable,
    interner: &mut TyInterner,
) -> TyId {
    // Clone the kind to release the borrow on `types` before recursive calls.
    let kind = types.get(type_id).kind.clone();
    match kind {
        HirTypeKind::Error => UNKNOWN_TY,

        HirTypeKind::Var(sym_id) => resolve_var(sym_id, symbols, interner),

        HirTypeKind::Function { param, ret } => {
            let p = ty_of_hir(param, types, symbols, interner);
            let r = ty_of_hir(ret, types, symbols, interner);
            interner.intern(Ty::Function { param: p, ret: r })
        }

        HirTypeKind::Apply { ctor, arg } => elab_apply(ctor, arg, types, symbols, interner),

        HirTypeKind::Optional(inner) => {
            let i = ty_of_hir(inner, types, symbols, interner);
            interner.intern(Ty::Optional(i))
        }

        HirTypeKind::Record { fields } => {
            let record_fields: Vec<RecordField> = fields
                .into_iter()
                .map(|(name, ty_id, fk)| RecordField {
                    name,
                    ty: ty_of_hir(ty_id, types, symbols, interner),
                    kind: match fk {
                        HirFieldKind::Required => FieldKind::Required,
                        HirFieldKind::Optional => FieldKind::Optional,
                    },
                })
                .collect();
            interner.intern(Ty::Record(record_fields))
        }

        HirTypeKind::Union { variants } => {
            let tys: Vec<TyId> = variants
                .into_iter()
                .map(|v| ty_of_hir(v, types, symbols, interner))
                .collect();
            interner.intern(Ty::Union(tys))
        }

        HirTypeKind::Variant { tag, fields } => {
            let elaborated: Vec<(String, TyId)> = fields
                .into_iter()
                .map(|(n, v)| (n, ty_of_hir(v, types, symbols, interner)))
                .collect();
            interner.intern(Ty::Variant {
                tag,
                fields: elaborated,
            })
        }

        HirTypeKind::SingletonAtom(atom) => interner.intern(Ty::Atom(atom)),

        HirTypeKind::SingletonLit(lit) => match lit {
            LitVal::Bool(_) => BOOL_TY,
            LitVal::None => NONE_TY,
            LitVal::Int(_) => INT_TY,
            LitVal::Float(_) => FLOAT_TY,
            LitVal::Text(_) => TEXT_TY,
            LitVal::Atom(s) => interner.intern(Ty::Atom(s)),
        },
    }
}

fn resolve_var(sym_id: SymbolId, symbols: &SymbolTable, interner: &mut TyInterner) -> TyId {
    if sym_id.is_error() {
        return UNKNOWN_TY;
    }
    let sym = symbols.get(sym_id);
    match sym.kind {
        SymbolKind::TypeParam => interner.intern(Ty::Param(sym_id.raw)),
        SymbolKind::TypeDef => match sym.name.as_str() {
            "Int" => INT_TY,
            "Float" => FLOAT_TY,
            "Text" => TEXT_TY,
            "Bool" => BOOL_TY,
            "None" | "none" => NONE_TY,
            _ => UNKNOWN_TY, // user-defined TypeDef — elaborated in Phase 6
        },
        _ => UNKNOWN_TY,
    }
}

fn elab_apply(
    ctor: HirTypeId,
    arg: HirTypeId,
    types: &Arena<HirType>,
    symbols: &SymbolTable,
    interner: &mut TyInterner,
) -> TyId {
    // Special-case `List T` → Ty::List(elab(T))
    let ctor_kind = types.get(ctor).kind.clone();
    if let HirTypeKind::Var(sym_id) = ctor_kind {
        if !sym_id.is_error() && symbols.get(sym_id).name == "List" {
            let arg_ty = ty_of_hir(arg, types, symbols, interner);
            return interner.intern(Ty::List(arg_ty));
        }
    }
    let ctor_ty = ty_of_hir(ctor, types, symbols, interner);
    let arg_ty = ty_of_hir(arg, types, symbols, interner);
    interner.intern(Ty::Apply {
        ctor: ctor_ty,
        arg: arg_ty,
    })
}
