use super::*;

impl<'a> Evaluator<'a> {
    // ── building top-level env ────────────────────────────────────────────────

    /// Build the top-level letrec environment from the file's declarations.
    ///
    /// The `top` frame is created first and shared across all thunks so that
    /// mutual recursion works.
    pub fn build_top_env(&self) -> Env {
        let top = Env::empty();
        // Seed prelude builtins (e.g. `print`). HIR seeds these into the root
        // scope first, so the lowest-id binding for each name is the prelude
        // one (a user lambda param sharing the name lives at a higher id and is
        // shadowed in a child frame at apply time).
        for &name in zutai_hir::BUILTIN_VALUE_NAMES {
            if let Some(builtin) = BuiltinFn::from_name(name)
                && let Some(index) = self.file.binding_names.iter().position(|n| n == name)
            {
                top.insert(
                    zutai_hir::BindingId(index as u32),
                    Thunk::ready(Value::Builtin(builtin)),
                );
            }
        }
        for &decl_id in &self.file.decls {
            let decl = self.decl(decl_id);
            match &decl.kind {
                ThirDeclKind::Value { value, .. } => {
                    // Deferred thunk stamped with this module so it evaluates
                    // against the correct arenas when forced.
                    let thunk = self.defer(*value, top.clone());
                    top.insert(decl.binding, thunk);
                }
                ThirDeclKind::Function { clauses, .. } => {
                    let arity = clauses.first().map(|c| c.patterns.len()).unwrap_or(0);
                    let closure = Closure {
                        binding: Some(decl.binding),
                        arity,
                        clauses: clauses.as_slice().into(),
                        env: top.clone(),
                        applied: Vec::new(),
                        home: self.active_module,
                    };
                    // Functions are pre-evaluated to closures.
                    top.insert(decl.binding, Thunk::ready(Value::Closure(Rc::new(closure))));
                }
                ThirDeclKind::TypeAlias { ty, .. } => {
                    // Type aliases are available as type values.
                    top.insert(
                        decl.binding,
                        Thunk::ready(Value::TypeValue(RuntimeType::new(self.active_module, *ty))),
                    );
                }
                // Constraint/witness decls contribute nothing to the eval environment
                // this increment; dictionary-passing elaboration is deferred.
                ThirDeclKind::Constraint { .. } | ThirDeclKind::Witness { .. } => {}
            }
        }
        top
    }
}
