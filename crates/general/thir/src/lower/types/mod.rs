use std::collections::{HashMap, HashSet};

use zutai_hir::{
    BindingId, BindingKind, HirEffectRow, HirRowTail, HirRowTailKind, HirTypeId, HirTypeKind,
    HirTypeRecordField, HirTypeTupleItem, HirUnionVariant,
};
use zutai_syntax::Span;

use crate::diagnostic::{RowOverlapItem, ThirDiagnostic, ThirDiagnosticKind};
use crate::ir::{
    EffectOp, EffectRow, Kind, RowTail, Type, TypeId, TypeKind, TypeRecordField, TypeTupleItem,
    UnionVariant,
};

use super::{Lowerer, RowSolution};

mod alias;
mod instantiate;
mod kind;
mod lower;
mod match_;
