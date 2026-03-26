//! Parallel frame scheduler.
//!
//! The [`Scheduler`] walks the DAG in topological order and dispatches
//! independent nodes concurrently using Rayon work-stealing.
//!
//! ## Execution model
//!
//! ```text
//!  Frame N
//!  ┌─────────┐     ┌─────────┐
//!  │ Read A  │     │ Read B  │   ← wave 0  (no inputs, run in parallel)
//!  └────┬────┘     └────┬────┘
//!       │               │
//!       └──────┬────────┘
//!              ▼
//!         ┌─────────┐
//!         │  Merge  │             ← wave 1  (depends on A and B)
//!         └────┬────┘
//!              ▼
//!         ┌─────────┐
//!         │  Grade  │             ← wave 2
//!         └─────────┘
//! ```
//!
//! Nodes whose entire dependency set has already been resolved form an
//! "execution wave". All nodes in a wave are submitted to the Rayon thread-pool
//! simultaneously via `par_iter()`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rayon::prelude::*;
use tracing::{debug, info, instrument};

use nodeflow_buffer::PixelBuffer;
use crate::{CompositorDag, CoreError, NodeId, ProcessContext};

/// Tuning knobs for the scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Override the Rayon global thread-pool size.  `None` = use all logical cores.
    pub thread_count: Option<usize>,
    /// When `true` every node's timing is logged at DEBUG level.
    pub trace_timing:  bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { thread_count: None, trace_timing: false }
    }
}

/// Caches the output of every node for one frame so downstream nodes don't
/// recompute shared dependencies.
type FrameCache = HashMap<NodeId, Arc<PixelBuffer>>;

pub struct Scheduler {
    cfg: SchedulerConfig,
}

impl Scheduler {
    pub fn new(cfg: SchedulerConfig) -> Self {
        if let Some(n) = cfg.thread_count {
            // Configure the global Rayon pool (must happen before first use).
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build_global()
                .ok(); // Ignore error if already initialised.
        }
        Self { cfg }
    }

    /// Execute all nodes in the DAG for `frame` and return the output of the
    /// **terminal node** (the node with no outgoing edges).
    ///
    /// # Errors
    /// Returns the first [`CoreError`] encountered.  The scheduler makes a
    /// best-effort attempt to fail fast.
    #[instrument(skip(self, dag), fields(frame))]
    pub fn render_frame(
        &self,
        dag:   &CompositorDag,
        frame: u64,
        fps:   f64,
    ) -> Result<Arc<PixelBuffer>, CoreError> {
        let order = dag.topological_order()?;
        info!("Rendering frame {frame}: {} nodes in topological order", order.len());

        // Shared, thread-safe output cache.
        let cache: Arc<Mutex<FrameCache>> = Arc::new(Mutex::new(HashMap::new()));

        // Group nodes into waves: a node enters wave `k` when the wave of
        // every predecessor is `< k`.
        let waves = build_waves(dag, &order)?;
        debug!("Execution plan: {} waves", waves.len());

        for (wave_idx, wave) in waves.iter().enumerate() {
            debug!("Wave {wave_idx}: {} node(s)", wave.len());

            // Collect results from the parallel wave into a Vec so we can
            // propagate the first error.
            let results: Vec<(NodeId, Result<Arc<PixelBuffer>, CoreError>)> = wave
                .par_iter()
                .map(|&node_id| {
                    let result = render_node(dag, node_id, frame, fps, &cache);
                    (node_id, result)
                })
                .collect();

            // Write successful results into the cache; return on first error.
            let mut cache_guard = cache.lock().unwrap();
            for (node_id, result) in results {
                match result {
                    Ok(buf)  => { cache_guard.insert(node_id, buf); }
                    Err(e)   => return Err(e),
                }
            }
        }

        // The terminal node is the last in topological order.
        let terminal_id = *order.last()
            .ok_or_else(|| CoreError::Scheduler("DAG is empty".into()))?;
        let cache_guard = cache.lock().unwrap();
        cache_guard.get(&terminal_id).cloned()
            .ok_or_else(|| CoreError::Scheduler(format!(
                "Terminal node {terminal_id} produced no output"
            )))
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Render a single node: resolve its inputs from the cache, call `process()`.
fn render_node(
    dag:    &CompositorDag,
    id:     NodeId,
    frame:  u64,
    fps:    f64,
    cache:  &Arc<Mutex<FrameCache>>,
) -> Result<Arc<PixelBuffer>, CoreError> {
    let node = dag.node(id)?;
    let input_specs = dag.inputs_of(id)?;

    // Build the inputs slice for `ProcessContext`.
    let input_count = node.metadata().input_count;
    let mut inputs: Vec<Option<Arc<PixelBuffer>>> = vec![None; input_count];

    {
        let cache_guard = cache.lock().unwrap();
        for (src_id, _src_port, dst_port) in &input_specs {
            if let Some(buf) = cache_guard.get(src_id) {
                if *dst_port < input_count {
                    inputs[*dst_port] = Some(Arc::clone(buf));
                }
            }
        }
    }

    let ctx = ProcessContext { inputs: &inputs, frame, fps };

    let t0 = std::time::Instant::now();
    let result = node.process(&ctx);
    debug!(
        node = %id,
        elapsed_us = t0.elapsed().as_micros(),
        "process() returned"
    );
    result
}

/// Assign each node in topological order to an execution wave.
/// Two nodes are in the same wave iff neither depends (transitively) on the
/// other for this frame.
fn build_waves(
    dag:   &CompositorDag,
    order: &[NodeId],
) -> Result<Vec<Vec<NodeId>>, CoreError> {
    let mut wave_of: HashMap<NodeId, usize> = HashMap::new();

    for &id in order {
        let inputs = dag.inputs_of(id)?;
        let max_predecessor_wave = inputs
            .iter()
            .filter_map(|(src_id, _, _)| wave_of.get(src_id).copied())
            .max()
            .unwrap_or(0);

        // If this node has predecessors, it belongs to wave `max + 1`.
        // Root nodes (no inputs) belong to wave 0.
        let my_wave = if inputs.is_empty() { 0 } else { max_predecessor_wave + 1 };
        wave_of.insert(id, my_wave);
    }

    // Group by wave index.
    let max_wave = wave_of.values().copied().max().unwrap_or(0);
    let mut waves = vec![Vec::new(); max_wave + 1];
    for &id in order {
        waves[wave_of[&id]].push(id);
    }

    Ok(waves)
}
