//! Core node abstraction.
//!
//! Every compositor operation (Merge, Grade, Blur, …) implements [`Node`].
//! Nodes are pure functions over [`PixelBuffer`]s: given a set of optional
//! input images they produce exactly one output image.  All state that affects
//! the output (knob values, lookup-tables, …) lives inside the concrete
//! implementation and must be `Send + Sync` so the scheduler can dispatch them
//! across threads freely.

use std::sync::Arc;
use nodeflow_buffer::PixelBuffer;
use serde::{Deserialize, Serialize};

/// Stable, unique identifier for a node inside a [`CompositorDag`].
/// Wraps a `u64` so that node IDs remain meaningful even after graph mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node#{}", self.0)
    }
}

/// Static description of a node that does *not* change at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub id:           NodeId,
    pub name:         String,
    /// Human-readable category shown in the node-picker UI (e.g. "Color", "Filter").
    pub category:     String,
    pub input_count:  usize,
    pub output_count: usize,
}

/// Everything a node needs to render one frame.
#[derive(Debug)]
pub struct ProcessContext<'a> {
    /// Resolved input buffers for this node (one per input port; `None` means
    /// the port is unconnected).
    pub inputs: &'a [Option<Arc<PixelBuffer>>],
    /// Frame number being rendered (for time-varying effects).
    pub frame:  u64,
    /// Frames-per-second of the composition timeline.
    pub fps:    f64,
}

/// The central trait every compositor node must implement.
///
/// # Thread-safety
/// Implementations **must** be `Send + Sync`.  The scheduler may call
/// `process()` from any Rayon worker thread.
pub trait Node: Send + Sync {
    fn metadata(&self) -> &NodeMetadata;

    /// Render this node for the given frame.
    ///
    /// The returned [`PixelBuffer`] is wrapped in an [`Arc`] so downstream
    /// nodes can share it without copying.
    fn process(
        &self,
        ctx: &ProcessContext<'_>,
    ) -> Result<Arc<PixelBuffer>, crate::CoreError>;
}

/// Convenience accessor so callers can write `node.id()` instead of
/// `node.metadata().id`.
pub trait NodeExt: Node {
    fn id(&self) -> NodeId     { self.metadata().id }
    fn name(&self) -> &str     { &self.metadata().name }
    fn input_count(&self) -> usize  { self.metadata().input_count }
    fn output_count(&self) -> usize { self.metadata().output_count }
}
impl<T: Node + ?Sized> NodeExt for T {}
