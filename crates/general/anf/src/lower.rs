//! DC → ANF lowering.
//!
//! Algorithm:
//! 1. Build global dependency graph + compute SCCs (Tarjan).
//! 2. Reverse SCC list → forward topological order.
//! 3. For each SCC decide let vs letrec and lower the node(s).
//! 4. Lower the root node as the module result.

use std::collections::HashMap;

use zutai_dataflow::{
    DataflowGraph, DfBuiltinOp, DfLit, DfNodeKind, DfPattern, DfTupleNodeItem, DfTuplePatItem,
    NodeId,
};

use crate::{
    AnfArm, AnfAtom, AnfBody, AnfDecl, AnfExpr, AnfModule, AnfPattern, AnfTupleItem,
    AnfTuplePatItem, scc,
};

// ── Body lowerer ─────────────────────────────────────────────────────────────

/// State for lowering one lambda (or module root) body.
struct BodyLowerer<'g> {
    graph: &'g DataflowGraph,
    /// Maps DC Bind NodeIds to their assigned ANF variable name.
    bind_names: &'g HashMap<NodeId, String>,
    /// Memoized results: if a node has already been lowered in this scope,
    /// reuse the atom (sharing). Reset when entering a nested lambda body.
    memo: HashMap<NodeId, AnfAtom>,
    /// Accumulated let-bindings for the current body, in order.
    bindings: Vec<(String, AnfExpr)>,
    /// Shared fresh name counter.
    counter: &'g mut u32,
}

impl<'g> BodyLowerer<'g> {
    fn new(
        graph: &'g DataflowGraph,
        bind_names: &'g HashMap<NodeId, String>,
        counter: &'g mut u32,
    ) -> Self {
        Self {
            graph,
            bind_names,
            memo: HashMap::new(),
            bindings: Vec::new(),
            counter,
        }
    }

    fn fresh(&mut self) -> String {
        let n = *self.counter;
        *self.counter += 1;
        format!("_anf{n}")
    }

    /// Lower `node_id` to an atom. If the node is complex, introduce a fresh
    /// binding and return a `Var` atom referencing it.
    fn lower_to_atom(&mut self, node_id: NodeId) -> AnfAtom {
        if let Some(atom) = self.memo.get(&node_id) {
            return atom.clone();
        }
        let atom = match &self.graph.nodes[node_id].kind {
            DfNodeKind::Lit(l) => AnfAtom::Lit(lower_lit(l)),
            DfNodeKind::Bind => {
                // Defensive fallback: well-typed v0 programs always pre-assign a
                // name via collect_all_bind_names (Lambda params) or lower_arm
                // (match-arm Bind nodes). The raw-index path fires only if a
                // future DC pass introduces a new Bind creation site not yet
                // covered by the pre-pass.
                let name = self
                    .bind_names
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_else(|| format!("_bind{}", node_id.into_raw().into_u32()));
                AnfAtom::Var(name)
            }
            DfNodeKind::GlobalRef(name) => AnfAtom::Global(name.clone()),
            _ => {
                let expr = self.lower_to_expr(node_id);
                let name = self.fresh();
                self.bindings.push((name.clone(), expr));
                let atom = AnfAtom::Var(name);
                self.memo.insert(node_id, atom.clone());
                return atom;
            }
        };
        // Cache atoms too (idempotent but avoids redundant lookups).
        self.memo.insert(node_id, atom.clone());
        atom
    }

    /// Lower `node_id` to an `AnfExpr`. The caller is responsible for
    /// introducing a binding for the result.
    fn lower_to_expr(&mut self, node_id: NodeId) -> AnfExpr {
        match self.graph.nodes[node_id].kind.clone() {
            DfNodeKind::Lit(l) => AnfExpr::Atom(AnfAtom::Lit(lower_lit(&l))),
            DfNodeKind::Bind => {
                let name = self
                    .bind_names
                    .get(&node_id)
                    .cloned()
                    .unwrap_or_else(|| format!("_bind{}", node_id.into_raw().into_u32()));
                AnfExpr::Atom(AnfAtom::Var(name))
            }
            DfNodeKind::GlobalRef(name) => AnfExpr::Atom(AnfAtom::Global(name)),
            DfNodeKind::Apply { func, arg } => {
                let f = self.lower_to_atom(func);
                let a = self.lower_to_atom(arg);
                AnfExpr::Apply { func: f, arg: a }
            }
            DfNodeKind::TyApp { poly, ty_args } => {
                let p = self.lower_to_atom(poly);
                AnfExpr::TyApp { poly: p, ty_args }
            }
            DfNodeKind::Lambda { param, body } => {
                // Assign the parameter's name before entering the body scope.
                let param_name = self
                    .bind_names
                    .get(&param)
                    .cloned()
                    .unwrap_or_else(|| format!("_bind{}", param.into_raw().into_u32()));
                let anf_body =
                    lower_body_in_new_scope(self.graph, self.bind_names, self.counter, body);
                AnfExpr::Lambda {
                    param: param_name,
                    body: anf_body,
                }
            }
            DfNodeKind::TyLam { ty_params, body } => {
                let anf_body =
                    lower_body_in_new_scope(self.graph, self.bind_names, self.counter, body);
                AnfExpr::TyLam {
                    ty_params,
                    body: anf_body,
                }
            }
            DfNodeKind::Record(fields) => {
                let anf_fields: Vec<(String, AnfAtom)> = fields
                    .into_iter()
                    .map(|(name, v)| (name, self.lower_to_atom(v)))
                    .collect();
                AnfExpr::Record(anf_fields)
            }
            DfNodeKind::RecordUpdate { base, updates } => {
                let base = self.lower_to_atom(base);
                let updates = updates
                    .into_iter()
                    .map(|(name, value)| (name, self.lower_to_atom(value)))
                    .collect();
                AnfExpr::RecordUpdate { base, updates }
            }
            DfNodeKind::Tuple(items) => {
                let anf_items: Vec<AnfTupleItem> = items
                    .into_iter()
                    .map(|item| match item {
                        DfTupleNodeItem::Named { name, value } => AnfTupleItem::Named {
                            name,
                            value: self.lower_to_atom(value),
                        },
                        DfTupleNodeItem::Positional(v) => {
                            AnfTupleItem::Positional(self.lower_to_atom(v))
                        }
                    })
                    .collect();
                AnfExpr::Tuple(anf_items)
            }
            DfNodeKind::List(elems) => {
                let anf_elems: Vec<AnfAtom> =
                    elems.into_iter().map(|e| self.lower_to_atom(e)).collect();
                AnfExpr::List(anf_elems)
            }
            DfNodeKind::Select { base, field } => {
                let b = self.lower_to_atom(base);
                AnfExpr::Select { base: b, field }
            }
            DfNodeKind::Match { scrutinee, arms } => {
                let s = self.lower_to_atom(scrutinee);
                let anf_arms: Vec<AnfArm> =
                    arms.into_iter().map(|arm| self.lower_arm(arm)).collect();
                AnfExpr::Match {
                    scrutinee: s,
                    arms: anf_arms,
                }
            }
            DfNodeKind::Coalesce { value, fallback } => {
                let v = self.lower_to_atom(value);
                let f = self.lower_to_atom(fallback);
                AnfExpr::Coalesce {
                    value: v,
                    fallback: f,
                }
            }
            DfNodeKind::Builtin(op, lhs, rhs) => {
                let l = self.lower_to_atom(lhs);
                let r = self.lower_to_atom(rhs);
                AnfExpr::Builtin {
                    op: lower_op(op),
                    lhs: l,
                    rhs: r,
                }
            }
            DfNodeKind::Variant(tag, inner) => {
                let v = self.lower_to_atom(inner);
                AnfExpr::Variant { tag, value: v }
            }
            // Defensive: Import and Error never appear in well-typed v0 programs.
            DfNodeKind::Import { .. } | DfNodeKind::Error => AnfExpr::Error,
        }
    }

    fn lower_arm(&mut self, arm: zutai_dataflow::DfArm) -> AnfArm {
        // Name all Bind nodes in the pattern BEFORE lowering guard/body.
        let mut pat_binds: HashMap<NodeId, String> = HashMap::new();
        collect_pat_bind_names(&arm.pattern, self.counter, &mut pat_binds);

        // Merge pattern bind names into our bind_names temporarily.
        // We do this by using a child lowerer with extended bind_names.
        // Since bind_names is immutable here, we track them separately
        // and look them up during pattern lowering.

        let anf_pattern = lower_dc_pattern(&arm.pattern, &pat_binds);

        // Build an extended bind map for this arm's body scope.
        let mut extended_binds: HashMap<NodeId, String> = self
            .bind_names
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect();
        extended_binds.extend(pat_binds);

        let guard_body = arm
            .guard
            .map(|g| lower_body_with_binds(self.graph, &extended_binds, self.counter, g));
        let body = lower_body_with_binds(self.graph, &extended_binds, self.counter, arm.body);

        AnfArm {
            pattern: anf_pattern,
            guard: guard_body,
            body,
        }
    }

    /// Finalise the current body: drain accumulated bindings, lower the root
    /// node to an atom, and return an `AnfBody`.
    fn finish(mut self, root: NodeId) -> AnfBody {
        let result = self.lower_to_atom(root);
        AnfBody {
            bindings: self.bindings,
            result,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn lower_lit(l: &DfLit) -> DfLit {
    l.clone()
}

fn lower_op(op: DfBuiltinOp) -> DfBuiltinOp {
    op
}

/// Lower a Lambda body in a fresh memo scope (closure starts a new scope).
fn lower_body_in_new_scope(
    graph: &DataflowGraph,
    bind_names: &HashMap<NodeId, String>,
    counter: &mut u32,
    body_node: NodeId,
) -> AnfBody {
    lower_body_with_binds(graph, bind_names, counter, body_node)
}

fn lower_body_with_binds(
    graph: &DataflowGraph,
    bind_names: &HashMap<NodeId, String>,
    counter: &mut u32,
    root: NodeId,
) -> AnfBody {
    let lowerer = BodyLowerer::new(graph, bind_names, counter);
    lowerer.finish(root)
}

/// Collect all `Bind` nodes in a DC pattern and assign them fresh names.
fn collect_pat_bind_names(pat: &DfPattern, counter: &mut u32, out: &mut HashMap<NodeId, String>) {
    match pat {
        DfPattern::Bind(id) => {
            let n = *counter;
            *counter += 1;
            out.insert(*id, format!("_bind{n}"));
        }
        DfPattern::Tuple(items) => {
            for item in items {
                let sub = match item {
                    DfTuplePatItem::Named { pattern, .. } => pattern,
                    DfTuplePatItem::Positional(p) => p,
                };
                collect_pat_bind_names(sub, counter, out);
            }
        }
        DfPattern::Record(fields) => {
            for (_, p) in fields {
                collect_pat_bind_names(p, counter, out);
            }
        }
        DfPattern::Variant(_, inner) => {
            collect_pat_bind_names(inner, counter, out);
        }
        DfPattern::Wildcard | DfPattern::Lit(_) | DfPattern::Atom(_) => {}
    }
}

fn lower_dc_pattern(pat: &DfPattern, pat_binds: &HashMap<NodeId, String>) -> AnfPattern {
    match pat {
        DfPattern::Wildcard => AnfPattern::Wildcard,
        DfPattern::Lit(l) => AnfPattern::Lit(l.clone()),
        DfPattern::Atom(s) => AnfPattern::Atom(s.clone()),
        DfPattern::Bind(id) => {
            let name = pat_binds
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("_bind{}", id.into_raw().into_u32()));
            AnfPattern::Bind(name)
        }
        DfPattern::Tuple(items) => {
            let anf_items: Vec<AnfTuplePatItem> = items
                .iter()
                .map(|item| match item {
                    DfTuplePatItem::Named { name, pattern } => AnfTuplePatItem::Named {
                        name: name.clone(),
                        pattern: lower_dc_pattern(pattern, pat_binds),
                    },
                    DfTuplePatItem::Positional(p) => {
                        AnfTuplePatItem::Positional(lower_dc_pattern(p, pat_binds))
                    }
                })
                .collect();
            AnfPattern::Tuple(anf_items)
        }
        DfPattern::Record(fields) => {
            let anf_fields: Vec<(String, AnfPattern)> = fields
                .iter()
                .map(|(name, p)| (name.clone(), lower_dc_pattern(p, pat_binds)))
                .collect();
            AnfPattern::Record(anf_fields)
        }
        DfPattern::Variant(tag, inner) => {
            AnfPattern::Variant(tag.clone(), Box::new(lower_dc_pattern(inner, pat_binds)))
        }
    }
}

// ── Pre-pass: collect all Bind node names ────────────────────────────────────

/// Walk the entire DC graph and pre-assign names to every Lambda `Bind` node.
///
/// Only Lambda params need pre-pass naming. Match-arm Bind nodes are
/// re-named locally inside `lower_arm` via `collect_pat_bind_names`, so
/// pre-assigning them here would produce unused names.
fn collect_all_bind_names(graph: &DataflowGraph, counter: &mut u32) -> HashMap<NodeId, String> {
    let mut names: HashMap<NodeId, String> = HashMap::new();
    for (_, node) in graph.nodes.iter() {
        if let DfNodeKind::Lambda { param, .. } = &node.kind {
            names.entry(*param).or_insert_with(|| {
                let n = *counter;
                *counter += 1;
                format!("_bind{n}")
            });
        }
    }
    names
}

// ── Module lowering ──────────────────────────────────────────────────────────

pub fn lower_dc(graph: &DataflowGraph) -> AnfModule {
    let mut counter: u32 = 0;

    // Pre-pass: name every Bind node in the graph.
    let bind_names = collect_all_bind_names(graph, &mut counter);

    // Stage 1+2: SCC analysis + topological sort.
    let (adj, self_loops) = scc::build_dep_graph(graph);
    // Tarjan gives forward topological order: sink SCCs (no dependencies) first.
    // Dependencies appear before their dependents — exactly the order we need.
    let sccs = scc::tarjan_sccs(&adj);

    // Stage 3: emit top-level decls for each SCC.
    let mut decls: Vec<AnfDecl> = Vec::new();

    for scc in sccs {
        if scc.len() == 1 {
            let name = &scc[0];
            let is_recursive = self_loops.contains(name.as_str());

            if let Some(&root) = graph.globals.get(name.as_str()) {
                let body = lower_body_with_binds(graph, &bind_names, &mut counter, root);
                if is_recursive {
                    decls.push(AnfDecl::Letrec {
                        bindings: vec![(name.clone(), body)],
                    });
                } else {
                    decls.push(AnfDecl::Let {
                        name: name.clone(),
                        body,
                    });
                }
            }
        } else {
            // Mutually recursive SCC → letrec.
            let bindings: Vec<(String, AnfBody)> = scc
                .iter()
                .filter_map(|name| {
                    graph.globals.get(name.as_str()).map(|&root| {
                        let body = lower_body_with_binds(graph, &bind_names, &mut counter, root);
                        (name.clone(), body)
                    })
                })
                .collect();
            if !bindings.is_empty() {
                decls.push(AnfDecl::Letrec { bindings });
            }
        }
    }

    // Lower the module root node.
    let root = lower_body_with_binds(graph, &bind_names, &mut counter, graph.root);

    AnfModule {
        decls,
        root,
        root_ty: graph.types[graph.nodes[graph.root].ty].clone(),
    }
}
