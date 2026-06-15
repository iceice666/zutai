use zutai_thir::ThirDeclId;

use crate::ir::TlcDeclId;

use super::Lowerer;

impl<'thir> Lowerer<'thir> {
    pub(super) fn lower_decl(&mut self, id: ThirDeclId) -> TlcDeclId {
        todo!("lower_decl: {:?}", id)
    }
}
