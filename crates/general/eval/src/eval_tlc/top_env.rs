use super::*;

impl<'a> TlcEvaluator<'a> {
    /// Build the top-level environment by evaluating all value decls in order.
    pub fn build_top_env(&self) -> Result<Env, EvalError> {
        self.build_top_env_from(Env::empty())
    }

    /// Build top-level declarations on top of a pre-seeded environment.
    pub fn build_top_env_from(&self, top: Env) -> Result<Env, EvalError> {
        for &decl_id in &self.module.decls {
            match &self.module.decl_arena[decl_id] {
                TlcDecl::Value { binding, body, .. } => {
                    top.insert(
                        *binding,
                        Thunk::tlc_deferred(*body, top.clone(), self.active_module),
                    );
                }
                TlcDecl::TypeAlias { .. } => {}
            }
        }
        Ok(top)
    }
}
