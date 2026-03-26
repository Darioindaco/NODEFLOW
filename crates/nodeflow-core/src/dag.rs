//! Directed Acyclic Graph of compositor nodes.
//!
//! Internally backed by [`petgraph::stable_graph::StableDiGraph`] so that node
//! removal does **not** invalidate existing [`NodeIndex`] handles.
//!
//! # Example
//! ```no_run
//! use nodeflow_core::{CompositorDag, NodeId};
//! // let mut dag = CompositorDag::new();
//! // let a = dag.add_node(my_node_a);
//! // let b = dag.add_node(my_node_b);
//! // dag.connect(a, 0, b, 0).unwrap();   // a's output 0 → b's input 0
//! ```

use std::collections::HashMap;

use petgraph::stable_graph::{StableDiGraph, NodeIndex};
use petgraph::algo::toposort;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::{Node, NodeId, CoreError};

/// A directed edge carries the port information for both endpoints.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Output port index on the source node.
    pub src_port: usize,
    /// Input  port index on the destination node.
    pub dst_port: usize,
}

/// Wrapper stored in each graph node; keeps the user-facing [`NodeId`]
/// alongside the boxed trait object.
struct NodeEntry {
    id:   NodeId,
    node: Box<dyn Node>,
}

/// The compositor DAG.
///
/// Nodes are stored as `Box<dyn Node>` trait objects so that arbitrary
/// implementations can coexist.  Edges carry [`Edge`] metadata describing
/// which ports are connected.
pub struct CompositorDag {
    graph:       StableDiGraph<NodeEntry, Edge>,
    /// Map from user-facing [`NodeId`] to the petgraph [`NodeIndex`].
    id_to_index: HashMap<NodeId, NodeIndex>,
    next_id:     u64,
}

impl CompositorDag {
    pub fn new() -> Self {
        Self {
            graph:       StableDiGraph::new(),
            id_to_index: HashMap::new(),
            next_id:     1,
        }
    }

    // ── Mutation ─────────────────────────────────────────────────────────────

    /// Add a node to the graph and return its stable [`NodeId`].
    pub fn add_node(&mut self, node: Box<dyn Node>) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        let idx = self.graph.add_node(NodeEntry { id, node });
        self.id_to_index.insert(id, idx);
        id
    }

    /// Remove a node and all its edges.
    pub fn remove_node(&mut self, id: NodeId) -> Result<(), CoreError> {
        let idx = self.index_of(id)?;
        self.graph.remove_node(idx);
        self.id_to_index.remove(&id);
        Ok(())
    }

    /// Connect `src`'s output port `src_port` to `dst`'s input port `dst_port`.
    ///
    /// Returns [`CoreError::CycleDetected`] if the edge would create a cycle.
    pub fn connect(
        &mut self,
        src:      NodeId,
        src_port: usize,
        dst:      NodeId,
        dst_port: usize,
    ) -> Result<(), CoreError> {
        let src_idx = self.index_of(src)?;
        let dst_idx = self.index_of(dst)?;

        // Temporarily add the edge and check for cycles.
        let edge_idx = self.graph.add_edge(src_idx, dst_idx, Edge { src_port, dst_port });
        if toposort(&self.graph, None).is_err() {
            // Cycle detected – roll back.
            self.graph.remove_edge(edge_idx);
            return Err(CoreError::CycleDetected {
                from: src.to_string(),
                to:   dst.to_string(),
            });
        }
        Ok(())
    }

    /// Disconnect an existing edge.
    pub fn disconnect(
        &mut self,
        src:      NodeId,
        src_port: usize,
        dst:      NodeId,
        dst_port: usize,
    ) -> Result<(), CoreError> {
        let src_idx = self.index_of(src)?;
        let dst_idx = self.index_of(dst)?;

        let edge = self.graph
            .edges_connecting(src_idx, dst_idx)
            .find(|e| e.weight().src_port == src_port && e.weight().dst_port == dst_port)
            .map(|e| e.id());

        match edge {
            Some(eid) => { self.graph.remove_edge(eid); Ok(()) }
            None      => Err(CoreError::NodeNotFound(format!(
                "edge {src}:{src_port} → {dst}:{dst_port}"
            ))),
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// Return a topologically-sorted list of node IDs (leaves first).
    pub fn topological_order(&self) -> Result<Vec<NodeId>, CoreError> {
        toposort(&self.graph, None)
            .map(|indices| indices.iter().map(|i| self.graph[*i].id).collect())
            .map_err(|_| CoreError::Scheduler("Cycle detected during toposort".into()))
    }

    /// Return the set of (src_id, src_port, dst_port) triples that feed into
    /// `dst`.
    pub fn inputs_of(&self, dst: NodeId)
        -> Result<Vec<(NodeId, usize, usize)>, CoreError>
    {
        let dst_idx = self.index_of(dst)?;
        Ok(self.graph
            .edges_directed(dst_idx, Direction::Incoming)
            .map(|e| {
                let src_id = self.graph[e.source()].id;
                (src_id, e.weight().src_port, e.weight().dst_port)
            })
            .collect())
    }

    /// Borrow the underlying [`Node`] trait object for `id`.
    pub fn node(&self, id: NodeId) -> Result<&dyn Node, CoreError> {
        let idx = self.index_of(id)?;
        Ok(self.graph[idx].node.as_ref())
    }

    pub fn node_count(&self) -> usize { self.graph.node_count() }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn index_of(&self, id: NodeId) -> Result<NodeIndex, CoreError> {
        self.id_to_index.get(&id).copied()
            .ok_or_else(|| CoreError::NodeNotFound(id.to_string()))
    }
}

impl Default for CompositorDag { fn default() -> Self { Self::new() } }
