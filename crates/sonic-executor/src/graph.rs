//! Parallel-execution graph: a DAG of [`ExecutableItem`]s rooted at a single
//! vertex whose triggers gate the whole graph.

use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::trigger::{TriggerDecl, TriggerDeclarer};

/// Opaque handle to a graph vertex. Returned by [`GraphBuilder::vertex`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Vertex(pub(crate) usize);

/// Internal graph storage.
#[allow(clippy::redundant_pub_crate)]
pub(crate) struct Graph {
    pub(crate) items: Vec<Box<dyn ExecutableItem>>,
    pub(crate) successors: Vec<Vec<usize>>,    // adjacency list
    pub(crate) in_degree: Vec<usize>,           // initial in-degree
    pub(crate) root: usize,
    pub(crate) decls: Vec<TriggerDecl>,
}

impl core::fmt::Debug for Graph {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Graph")
            .field("n_items", &self.items.len())
            .field("successors", &self.successors)
            .field("in_degree", &self.in_degree)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Builder for a graph.
pub struct GraphBuilder {
    items: Vec<Box<dyn ExecutableItem>>,
    edges: Vec<(usize, usize)>,
    root: Option<usize>,
}

impl GraphBuilder {
    pub(crate) fn new() -> Self {
        Self { items: Vec::new(), edges: Vec::new(), root: None }
    }

    /// Add a vertex; returns its handle.
    pub fn vertex<I: ExecutableItem>(&mut self, item: I) -> Vertex {
        let idx = self.items.len();
        self.items.push(Box::new(item));
        Vertex(idx)
    }

    /// Add a directed edge `from -> to`.
    pub fn edge(&mut self, from: Vertex, to: Vertex) -> &mut Self {
        self.edges.push((from.0, to.0));
        self
    }

    /// Designate the root vertex (whose triggers gate the graph).
    pub fn root(&mut self, v: Vertex) -> &mut Self {
        self.root = Some(v.0);
        self
    }

    /// Build, validating connectedness, acyclicity, and exactly-one root.
    pub(crate) fn finish(mut self) -> Result<Graph, ExecutorError> {
        let n = self.items.len();
        if n == 0 {
            return Err(ExecutorError::InvalidGraph("graph has no vertices".into()));
        }
        let root = self
            .root
            .ok_or_else(|| ExecutorError::InvalidGraph("no root vertex set".into()))?;
        if root >= n {
            return Err(ExecutorError::InvalidGraph("root index out of bounds".into()));
        }

        let mut successors = vec![Vec::<usize>::new(); n];
        let mut in_degree = vec![0_usize; n];
        for &(from, to) in &self.edges {
            if from >= n || to >= n {
                return Err(ExecutorError::InvalidGraph("edge index out of bounds".into()));
            }
            if from == to {
                return Err(ExecutorError::InvalidGraph("self-loops are not allowed".into()));
            }
            successors[from].push(to);
            in_degree[to] += 1;
        }

        // Acyclicity via Kahn's algorithm — clone in_degree because we mutate.
        let mut k_in = in_degree.clone();
        let mut queue: Vec<usize> = k_in
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut visited = 0_usize;
        while let Some(u) = queue.pop() {
            visited += 1;
            for &v in &successors[u] {
                k_in[v] -= 1;
                if k_in[v] == 0 {
                    queue.push(v);
                }
            }
        }
        if visited != n {
            return Err(ExecutorError::InvalidGraph("graph contains a cycle".into()));
        }

        // Reachability from root (DFS).
        let mut reach = vec![false; n];
        let mut stack = vec![root];
        while let Some(u) = stack.pop() {
            if reach[u] { continue; }
            reach[u] = true;
            for &v in &successors[u] {
                stack.push(v);
            }
        }
        if reach.iter().any(|r| !*r) {
            return Err(ExecutorError::InvalidGraph(
                "every vertex must be reachable from the root".into(),
            ));
        }

        // Root's triggers gate the graph.
        let mut decl = TriggerDeclarer::new_internal();
        self.items[root].declare_triggers(&mut decl)?;
        let decls = decl.into_decls();

        // Warn if non-root vertices declared triggers (ignored).
        for (i, body) in self.items.iter_mut().enumerate() {
            if i == root { continue; }
            let mut spurious = TriggerDeclarer::new_internal();
            let _ = body.declare_triggers(&mut spurious);
            if !spurious.is_empty() {
                #[cfg(feature = "tracing")]
                tracing::warn!(target: "sonic-executor", vertex = i,
                    "non-root graph vertex declared triggers; ignored");
            }
        }

        Ok(Graph {
            items: self.items,
            successors,
            in_degree,
            root,
            decls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{item, ControlFlow};

    #[test]
    fn empty_graph_rejected() {
        let b = GraphBuilder::new();
        let err = b.finish().expect_err("empty graph");
        assert!(format!("{err}").contains("no vertices"));
    }

    #[test]
    fn missing_root_rejected() {
        let mut b = GraphBuilder::new();
        b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let err = b.finish().expect_err("missing root");
        assert!(format!("{err}").contains("no root"));
    }

    #[test]
    fn cycle_rejected() {
        let mut b = GraphBuilder::new();
        let a = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let v = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        b.edge(a, v).edge(v, a).root(a);
        let err = b.finish().expect_err("cycle");
        assert!(format!("{err}").contains("cycle"));
    }

    #[test]
    fn unreachable_vertex_rejected() {
        let mut b = GraphBuilder::new();
        let a = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let _orphan = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        b.root(a);
        let err = b.finish().expect_err("unreachable");
        assert!(format!("{err}").contains("reachable"));
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn diamond_graph_builds() {
        let mut b = GraphBuilder::new();
        let r  = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let l  = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let rt = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let m  = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        b.edge(r, l).edge(r, rt).edge(l, m).edge(rt, m).root(r);
        let g = b.finish().expect("diamond");
        assert_eq!(g.successors[r.0], vec![l.0, rt.0]);
        assert_eq!(g.in_degree[m.0], 2);
    }
}
