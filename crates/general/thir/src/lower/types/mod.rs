use rustc_hash::{FxHashMap, FxHashSet};

use zutai_hir::{
    BindingId, BindingKind, HirEffectRow, HirRowTail, HirRowTailKind, HirTypeId, HirTypeKind,
    HirTypeRecordField, HirTypeTupleItem, HirUnionVariant,
};
use zutai_syntax::Span;

use crate::diagnostic::{RowOverlapItem, ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    EffectOp, EffectRow, Kind, RowTail, Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
    UnionVariant, UniverseLevel,
};

use super::{Lowerer, RowSolution};

mod alias;
mod apply;
mod collect;
mod constructors;
mod generalize;
mod instantiate;
mod kind;
mod levels;
mod lower;
mod match_;
mod patch;

pub(in crate::lower) use match_::WrapperKind;
