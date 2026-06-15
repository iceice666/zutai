use zutai_thir::ThirExprId;

use crate::ir::TlcExprId;

use super::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_expr(&mut self, id: ThirExprId) -> TlcExprId {
        todo!("lower_expr: {:?}", id)
    }

    pub(super) fn lower_pat(&mut self, id: zutai_thir::ThirPatId) -> crate::ir::TlcPat {
        todo!("lower_pat: {:?}", id)
    }
}
