//! NodeFlow compositor – integration demo.
//!
//! Builds the following composition and renders frame 0:
//!
//! ```text
//!  ┌──────────────┐     ┌──────────────┐
//!  │  SolidColor  │     │  SolidColor  │
//!  │  (red 50%)   │     │  (blue 50%)  │
//!  └──────┬───────┘     └──────┬───────┘
//!         │    input0          │    input1
//!         └──────────┬─────────┘
//!                    ▼
//!               ┌─────────┐
//!               │  Merge  │   (over blend)
//!               └────┬────┘
//!                    ▼
//!               ┌─────────┐
//!               │  Grade  │   (+0.1 lift on all channels)
//!               └─────────┘
//! ```
//!
//! After rendering we print the colour of the centre pixel.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

use nodeflow_buffer::{PixelBuffer, PixelFormat, BlendMode, apply_blend, apply_grade};
use nodeflow_core::{
    CompositorDag, Node, NodeId, NodeMetadata, ProcessContext,
    Scheduler, SchedulerConfig, CoreError,
};

// ── Built-in node definitions ─────────────────────────────────────────────────

/// Outputs a solid RGBA colour at whatever resolution is requested.
struct SolidColorNode {
    meta:  NodeMetadata,
    color: [f32; 4],
    width: u32,
    height: u32,
}

impl SolidColorNode {
    fn new(id: NodeId, name: &str, color: [f32; 4], width: u32, height: u32) -> Self {
        Self {
            meta: NodeMetadata {
                id,
                name:         name.into(),
                category:     "Generator".into(),
                input_count:  0,
                output_count: 1,
            },
            color,
            width,
            height,
        }
    }
}

impl Node for SolidColorNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }

    fn process(&self, _ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let mut buf = PixelBuffer::new(self.width, self.height, PixelFormat::LinearRgba)
            .map_err(|e| CoreError::ProcessFailed(self.meta.name.clone(), e.to_string()))?;

        // Fill every pixel with `color`.
        for y in 0..self.height {
            for x in 0..self.width {
                buf.set_pixel(x, y, self.color);
            }
        }
        Ok(Arc::new(buf))
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Composites two inputs using a [`BlendMode`].
struct MergeNode {
    meta: NodeMetadata,
    mode: BlendMode,
}

impl MergeNode {
    fn new(id: NodeId, name: &str, mode: BlendMode) -> Self {
        Self {
            meta: NodeMetadata {
                id,
                name:         name.into(),
                category:     "Composite".into(),
                input_count:  2,
                output_count: 1,
            },
            mode,
        }
    }
}

impl Node for MergeNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }

    fn process(&self, ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let fail = |msg: &str| CoreError::ProcessFailed(self.meta.name.clone(), msg.into());

        let bg = ctx.inputs.get(0).and_then(|o| o.as_ref())
            .ok_or_else(|| fail("input 0 (background) is not connected"))?;
        let fg = ctx.inputs.get(1).and_then(|o| o.as_ref())
            .ok_or_else(|| fail("input 1 (foreground) is not connected"))?;

        // Clone the background; `apply_blend` operates in-place on `dst`.
        let mut out = bg.clone_buffer()
            .map_err(|e| fail(&e.to_string()))?;
        apply_blend(&mut out, fg, self.mode)
            .map_err(|e| fail(&e.to_string()))?;

        Ok(Arc::new(out))
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Applies a simple lift/gain/gamma colour grade.
struct GradeNode {
    meta:  NodeMetadata,
    lift:  [f32; 4],
    gain:  [f32; 4],
    gamma: [f32; 4],
}

impl GradeNode {
    fn new(
        id:    NodeId,
        name:  &str,
        lift:  [f32; 4],
        gain:  [f32; 4],
        gamma: [f32; 4],
    ) -> Self {
        Self {
            meta: NodeMetadata {
                id,
                name:         name.into(),
                category:     "Color".into(),
                input_count:  1,
                output_count: 1,
            },
            lift, gain, gamma,
        }
    }
}

impl Node for GradeNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }

    fn process(&self, ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let fail = |msg: &str| CoreError::ProcessFailed(self.meta.name.clone(), msg.into());
        let src  = ctx.inputs.get(0).and_then(|o| o.as_ref())
            .ok_or_else(|| fail("input 0 is not connected"))?;

        let mut out = src.clone_buffer().map_err(|e| fail(&e.to_string()))?;
        apply_grade(&mut out, self.lift, self.gain, self.gamma)
            .map_err(|e| fail(&e.to_string()))?;

        Ok(Arc::new(out))
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    // Enable RUST_LOG=info (or debug) to see scheduler traces.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()
            .add_directive("nodeflow=info".parse()?))
        .init();

    const W: u32 = 1920;
    const H: u32 = 1080;

    // ── Build the DAG ─────────────────────────────────────────────────────────
    let mut dag = CompositorDag::new();

    // Allocate stable IDs up-front so we can wire edges.
    let id_red  = NodeId(1);
    let id_blue = NodeId(2);
    let id_merge = NodeId(3);
    let id_grade = NodeId(4);

    dag.add_node(Box::new(SolidColorNode::new(
        id_red, "Red50", [0.8, 0.0, 0.0, 0.5], W, H,
    )));
    dag.add_node(Box::new(SolidColorNode::new(
        id_blue, "Blue50", [0.0, 0.0, 0.9, 0.5], W, H,
    )));
    dag.add_node(Box::new(MergeNode::new(id_merge, "Merge", BlendMode::Over)));
    dag.add_node(Box::new(GradeNode::new(
        id_grade, "Grade",
        [0.05, 0.05, 0.05, 0.0],  // lift
        [1.0,  1.0,  1.0,  1.0],  // gain
        [1.0,  1.0,  1.0,  1.0],  // gamma
    )));

    // Wire: Red50 → Merge input 0 (background)
    dag.connect(id_red,   0, id_merge, 0)?;
    // Wire: Blue50 → Merge input 1 (foreground)
    dag.connect(id_blue,  0, id_merge, 1)?;
    // Wire: Merge → Grade input 0
    dag.connect(id_merge, 0, id_grade, 0)?;

    info!(
        "DAG built: {} nodes, topo-order = {:?}",
        dag.node_count(),
        dag.topological_order()?
    );

    // ── Render frame 0 ───────────────────────────────────────────────────────
    let scheduler = Scheduler::new(SchedulerConfig {
        thread_count: None,   // use all cores
        trace_timing: true,
    });

    let t0     = std::time::Instant::now();
    let output = scheduler.render_frame(&dag, 0, 24.0)?;
    let elapsed = t0.elapsed();

    info!(
        "Render complete in {:.2}ms  ({}×{} RGBA f32 = {:.1} MiB)",
        elapsed.as_secs_f64() * 1000.0,
        output.width(), output.height(),
        (output.len() * 4) as f64 / (1024.0 * 1024.0),
    );

    // ── Inspect centre pixel ─────────────────────────────────────────────────
    let cx = output.width()  / 2;
    let cy = output.height() / 2;
    let [r, g, b, a] = output.get_pixel(cx, cy);
    println!("Centre pixel ({cx},{cy}):  R={r:.4}  G={g:.4}  B={b:.4}  A={a:.4}");

    // ── Quick buffer-ops benchmark ───────────────────────────────────────────
    println!("\n── Buffer ops benchmark ──");
    {
        use nodeflow_buffer::apply_multiply_scalar;

        let mut buf = PixelBuffer::new(3840, 2160, PixelFormat::LinearRgba)?;
        // Pre-fill with 0.5
        for v in buf.as_slice_mut() { *v = 0.5; }

        let t = std::time::Instant::now();
        for _ in 0..10 {
            apply_multiply_scalar(&mut buf, 0.99)?;
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "10× scale on 4K RGBA f32 ({:.0} Mfloats):  {:.2} ms total  ({:.2} ms/frame)",
            (buf.len() as f64) / 1e6, ms, ms / 10.0
        );
    }

    Ok(())
}
