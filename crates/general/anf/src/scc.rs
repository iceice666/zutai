//! Tarjan's SCC algorithm on the global dependency graph.
//!
//! Edges: global G → global H when G's node sub-graph contains `GlobalRef(H)`.
//! Tarjan returns SCCs in reverse topological order; callers must reverse the
//! result to get forward topological order (dependencies before dependents).

use rustc_hash::{FxHashMap, FxHashSet};

use zutai_dataflow::{
    DataflowGraph, DfNodeKind, DfPattern, DfTupleNodeItem, DfTuplePatItem, NodeId,
};

// ── Dependency graph extraction ───────────────────────────────────────────────

fn collect_global_refs(
    graph: &DataflowGraph,
    root: NodeId,
    out: &mut FxHashSet<String>,
    visited: &mut FxHashSet<NodeId>,
) {
    if !visited.insert(root) {
        return;
    }
    match &graph.nodes[root].kind {
        DfNodeKind::GlobalRef(name) => {
            out.insert(name.clone());
        }
        DfNodeKind::Lambda { param: _, body } => {
            collect_global_refs(graph, *body, out, visited);
        }
        DfNodeKind::Apply { func, arg } => {
            collect_global_refs(graph, *func, out, visited);
            collect_global_refs(graph, *arg, out, visited);
        }
        DfNodeKind::HostPrint { arg } => {
            collect_global_refs(graph, *arg, out, visited);
        }
        DfNodeKind::HostOp { arg, .. } => {
            collect_global_refs(graph, *arg, out, visited);
        }
        DfNodeKind::TyLam { ty_params: _, body } => {
            collect_global_refs(graph, *body, out, visited);
        }
        DfNodeKind::TyApp { poly, ty_args: _ } => {
            collect_global_refs(graph, *poly, out, visited);
        }
        DfNodeKind::Record(fields) => {
            for (_, v) in fields {
                collect_global_refs(graph, *v, out, visited);
            }
        }
        DfNodeKind::RecordUpdate { base, updates } => {
            collect_global_refs(graph, *base, out, visited);
            for (_, _, value) in updates {
                collect_global_refs(graph, *value, out, visited);
            }
        }
        DfNodeKind::Tuple(items) => {
            let ids: Vec<NodeId> = items
                .iter()
                .map(|item| match item {
                    DfTupleNodeItem::Named { value, .. } => *value,
                    DfTupleNodeItem::Positional(id) => *id,
                })
                .collect();
            for id in ids {
                collect_global_refs(graph, id, out, visited);
            }
        }
        DfNodeKind::List(elems) => {
            let elems: Vec<NodeId> = elems.clone();
            for e in elems {
                collect_global_refs(graph, e, out, visited);
            }
        }
        DfNodeKind::Select { base, .. } => {
            collect_global_refs(graph, *base, out, visited);
        }
        DfNodeKind::Match { scrutinee, arms } => {
            collect_global_refs(graph, *scrutinee, out, visited);
            // Clone arm data to avoid borrow conflict with arena
            let arm_data: Vec<(Option<NodeId>, NodeId, DfPattern)> = arms
                .iter()
                .map(|arm| (arm.guard, arm.body, arm.pattern.clone()))
                .collect();
            for (guard, body, pattern) in arm_data {
                if let Some(g) = guard {
                    collect_global_refs(graph, g, out, visited);
                }
                collect_global_refs(graph, body, out, visited);
                collect_pat_refs(graph, &pattern, out, visited);
            }
        }
        DfNodeKind::Coalesce { value, fallback } => {
            collect_global_refs(graph, *value, out, visited);
            collect_global_refs(graph, *fallback, out, visited);
        }
        DfNodeKind::Builtin(_, lhs, rhs) => {
            collect_global_refs(graph, *lhs, out, visited);
            collect_global_refs(graph, *rhs, out, visited);
        }
        DfNodeKind::ListPrim { args, .. }
        | DfNodeKind::NumPrim { args, .. }
        | DfNodeKind::TextPrim { args, .. } => {
            for arg in args {
                collect_global_refs(graph, *arg, out, visited);
            }
        }
        DfNodeKind::Sequence(items) => {
            for item in items {
                collect_global_refs(graph, *item, out, visited);
            }
        }
        DfNodeKind::Variant { value, .. } => {
            collect_global_refs(graph, *value, out, visited);
        }
        // Leaves: Lit, Bind, Import, Error — no children to visit
        DfNodeKind::Lit(_) | DfNodeKind::Bind | DfNodeKind::Import { .. } | DfNodeKind::Error => {}
    }
}

fn collect_pat_refs(
    graph: &DataflowGraph,
    pat: &DfPattern,
    out: &mut FxHashSet<String>,
    visited: &mut FxHashSet<NodeId>,
) {
    match pat {
        DfPattern::Bind(id) => collect_global_refs(graph, *id, out, visited),
        DfPattern::Tuple(items) => {
            let subs: Vec<DfPattern> = items
                .iter()
                .map(|item| match item {
                    DfTuplePatItem::Named { pattern, .. } => pattern.clone(),
                    DfTuplePatItem::Positional(p) => p.clone(),
                })
                .collect();
            for sub in subs {
                collect_pat_refs(graph, &sub, out, visited);
            }
        }
        DfPattern::ListCons { head, tail } => {
            collect_pat_refs(graph, head, out, visited);
            collect_pat_refs(graph, tail, out, visited);
        }
        DfPattern::Record(fields) => {
            let subs: Vec<DfPattern> = fields.iter().map(|(_, _, p)| p.clone()).collect();
            for sub in subs {
                collect_pat_refs(graph, &sub, out, visited);
            }
        }
        DfPattern::Variant { pattern, .. } => {
            let inner = pattern.as_ref().clone();
            collect_pat_refs(graph, &inner, out, visited);
        }
        DfPattern::Wildcard | DfPattern::Lit(_) | DfPattern::Atom(_) | DfPattern::ListNil => {}
    }
}

/// Build the global dependency graph.
///
/// Returns:
/// - `adj`: `name → set of referenced global names` (self-refs excluded).
/// - `self_loops`: set of global names that have a self-edge.
pub fn build_dep_graph(
    graph: &DataflowGraph,
) -> (FxHashMap<String, FxHashSet<String>>, FxHashSet<String>) {
    let mut adj: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    let mut self_loops: FxHashSet<String> = FxHashSet::default();

    for (name, &root) in &graph.globals {
        let mut all_refs: FxHashSet<String> = FxHashSet::default();
        let mut visited: FxHashSet<NodeId> = FxHashSet::default();
        collect_global_refs(graph, root, &mut all_refs, &mut visited);

        if all_refs.remove(name.as_str()) {
            self_loops.insert(name.clone());
        }
        // Keep only refs that are valid globals (in case of stray refs, which
        // the DC validator would have caught).
        all_refs.retain(|r| graph.globals.contains_key(r.as_str()));
        adj.insert(name.clone(), all_refs);
    }

    (adj, self_loops)
}

// ── Tarjan's SCC ─────────────────────────────────────────────────────────────

struct TarjanState<'a> {
    adj: &'a FxHashMap<String, FxHashSet<String>>,
    index: FxHashMap<String, usize>,
    lowlink: FxHashMap<String, usize>,
    on_stack: FxHashSet<String>,
    stack: Vec<String>,
    counter: usize,
    sccs: Vec<Vec<String>>,
}

impl<'a> TarjanState<'a> {
    fn strongconnect(&mut self, v: String) {
        self.index.insert(v.clone(), self.counter);
        self.lowlink.insert(v.clone(), self.counter);
        self.counter += 1;
        self.stack.push(v.clone());
        self.on_stack.insert(v.clone());

        let neighbors: Vec<String> = self
            .adj
            .get(&v)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        for w in neighbors {
            if !self.index.contains_key(&w) {
                self.strongconnect(w.clone());
                let w_ll = self.lowlink[&w];
                let v_ll = self.lowlink[&v];
                self.lowlink.insert(v.clone(), v_ll.min(w_ll));
            } else if self.on_stack.contains(&w) {
                let w_idx = self.index[&w];
                let v_ll = self.lowlink[&v];
                self.lowlink.insert(v.clone(), v_ll.min(w_idx));
            }
        }

        if self.lowlink[&v] == self.index[&v] {
            let mut scc = Vec::new();
            loop {
                let w = self.stack.pop().expect("stack non-empty");
                self.on_stack.remove(&w);
                scc.push(w.clone());
                if w == v {
                    break;
                }
            }
            self.sccs.push(scc);
        }
    }
}

/// Compute SCCs using Tarjan's algorithm.
///
/// Returns SCCs in **forward topological order**: if SCC X depends on SCC Y
/// (has an edge to Y), then Y appears *before* X. The first SCC has no
/// outgoing edges to later SCCs — it is a leaf in the condensation DAG and
/// may safely be emitted first. No reversal is needed by callers.
pub fn tarjan_sccs(adj: &FxHashMap<String, FxHashSet<String>>) -> Vec<Vec<String>> {
    let mut state = TarjanState {
        adj,
        index: FxHashMap::default(),
        lowlink: FxHashMap::default(),
        on_stack: FxHashSet::default(),
        stack: Vec::new(),
        counter: 0,
        sccs: Vec::new(),
    };

    // Stable iteration order for deterministic output.
    let mut nodes: Vec<String> = adj.keys().cloned().collect();
    nodes.sort();
    for v in nodes {
        if !state.index.contains_key(&v) {
            state.strongconnect(v);
        }
    }

    state.sccs
}
