//! NodeFlow compositor – headless integration demo.

mod nodes;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

use nodeflow_buffer::{PixelBuffer, PixelFormat, BlendMode, apply_multiply_scalar};
use nodeflow_core::{CompositorDag, NodeId, Scheduler, SchedulerConfig};

use nodes::{SolidColorNode, MergeNode, GradeNode};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env()
            .add_directive("nodeflow=info".parse()?))
        .init();

    const W: u32 = 1920;
    const H: u32 = 1080;

    let mut dag = CompositorDag::new();

    let id_red   = NodeId(1);
    let id_blue  = NodeId(2);
    let id_merge = NodeId(3);
    let id_grade = NodeId(4);

    dag.add_node(Box::new(SolidColorNode::new(id_red,  "Red50",  [0.8, 0.0, 0.0, 0.5], W, H)));
    dag.add_node(Box::new(SolidColorNode::new(id_blue, "Blue50", [0.0, 0.0, 0.9, 0.5], W, H)));
    dag.add_node(Box::new(MergeNode::new(id_merge, "Merge", BlendMode::Over)));
    dag.add_node(Box::new(GradeNode::new(
        id_grade, "Grade",
        [0.05, 0.05, 0.05, 0.0],
        [1.0,  1.0,  1.0,  1.0],
        [1.0,  1.0,  1.0,  1.0],
    )));

    dag.connect(id_red,   0, id_merge, 0)?;
    dag.connect(id_blue,  0, id_merge, 1)?;
    dag.connect(id_merge, 0, id_grade, 0)?;

    info!("DAG built: {} nodes, topo-order = {:?}", dag.node_count(), dag.topological_order()?);

    let scheduler = Scheduler::new(SchedulerConfig { thread_count: None, trace_timing: true });

    let t0      = std::time::Instant::now();
    let output  = scheduler.render_frame(&dag, 0, 24.0)?;
    let elapsed = t0.elapsed();

    info!(
        "Render complete in {:.2}ms  ({}×{} RGBA f32 = {:.1} MiB)",
        elapsed.as_secs_f64() * 1000.0,
        output.width(), output.height(),
        (output.len() * 4) as f64 / (1024.0 * 1024.0),
    );

    let [r, g, b, a] = output.get_pixel(output.width() / 2, output.height() / 2);
    println!("Centre pixel:  R={r:.4}  G={g:.4}  B={b:.4}  A={a:.4}");

    // Benchmark
    let mut buf = PixelBuffer::new(3840, 2160, PixelFormat::LinearRgba)?;
    for v in buf.as_slice_mut() { *v = 0.5; }
    let t = std::time::Instant::now();
    for _ in 0..10 { apply_multiply_scalar(&mut buf, 0.99)?; }
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("10× scale 4K:  {:.2} ms total  ({:.2} ms/frame)", ms, ms / 10.0);

    Ok(())
}
