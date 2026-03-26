//! Ingebouwde compositor-nodes gedeeld door de headless demo én de UI.

use std::sync::Arc;

use nodeflow_buffer::{PixelBuffer, PixelFormat, BlendMode, apply_blend, apply_grade};
use nodeflow_core::{Node, NodeId, NodeMetadata, ProcessContext, CoreError};

// ── SolidColorNode ────────────────────────────────────────────────────────────

pub struct SolidColorNode {
    pub meta:   NodeMetadata,
    pub color:  [f32; 4],
    pub width:  u32,
    pub height: u32,
}

impl SolidColorNode {
    pub fn new(id: NodeId, name: &str, color: [f32; 4], width: u32, height: u32) -> Self {
        Self {
            meta: NodeMetadata {
                id, name: name.into(), category: "Generator".into(),
                input_count: 0, output_count: 1,
            },
            color, width, height,
        }
    }
}

impl Node for SolidColorNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }
    fn process(&self, _ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let mut buf = PixelBuffer::new(self.width, self.height, PixelFormat::LinearRgba)
            .map_err(|e| CoreError::ProcessFailed(self.meta.name.clone(), e.to_string()))?;
        for y in 0..self.height { for x in 0..self.width { buf.set_pixel(x, y, self.color); } }
        Ok(Arc::new(buf))
    }
}

// ── MergeNode ─────────────────────────────────────────────────────────────────

pub struct MergeNode {
    pub meta: NodeMetadata,
    pub mode: BlendMode,
}

impl MergeNode {
    pub fn new(id: NodeId, name: &str, mode: BlendMode) -> Self {
        Self {
            meta: NodeMetadata {
                id, name: name.into(), category: "Composite".into(),
                input_count: 2, output_count: 1,
            },
            mode,
        }
    }
}

impl Node for MergeNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }
    fn process(&self, ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let fail = |msg: &str| CoreError::ProcessFailed(self.meta.name.clone(), msg.into());
        let bg = ctx.inputs.get(0).and_then(|o| o.as_ref()).ok_or_else(|| fail("input 0 (Bg) niet verbonden"))?;
        let fg = ctx.inputs.get(1).and_then(|o| o.as_ref()).ok_or_else(|| fail("input 1 (Fg) niet verbonden"))?;
        let mut out = bg.clone_buffer().map_err(|e| fail(&e.to_string()))?;
        apply_blend(&mut out, fg, self.mode).map_err(|e| fail(&e.to_string()))?;
        Ok(Arc::new(out))
    }
}

// ── GradeNode ─────────────────────────────────────────────────────────────────

pub struct GradeNode {
    pub meta:  NodeMetadata,
    pub lift:  [f32; 4],
    pub gain:  [f32; 4],
    pub gamma: [f32; 4],
}

impl GradeNode {
    pub fn new(id: NodeId, name: &str, lift: [f32; 4], gain: [f32; 4], gamma: [f32; 4]) -> Self {
        Self {
            meta: NodeMetadata {
                id, name: name.into(), category: "Color".into(),
                input_count: 1, output_count: 1,
            },
            lift, gain, gamma,
        }
    }
}

impl Node for GradeNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }
    fn process(&self, ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let fail = |msg: &str| CoreError::ProcessFailed(self.meta.name.clone(), msg.into());
        let src = ctx.inputs.get(0).and_then(|o| o.as_ref()).ok_or_else(|| fail("input 0 niet verbonden"))?;
        let mut out = src.clone_buffer().map_err(|e| fail(&e.to_string()))?;
        apply_grade(&mut out, self.lift, self.gain, self.gamma).map_err(|e| fail(&e.to_string()))?;
        Ok(Arc::new(out))
    }
}

// ── FileReadNode ──────────────────────────────────────────────────────────────

pub struct FileReadNode {
    pub meta: NodeMetadata,
    pub path: String,
}

impl FileReadNode {
    pub fn new(id: NodeId, name: &str, path: &str) -> Self {
        Self {
            meta: NodeMetadata {
                id, name: name.into(), category: "Input".into(),
                input_count: 0, output_count: 1,
            },
            path: path.into(),
        }
    }
}

impl Node for FileReadNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }

    fn process(&self, _ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        let fail = |msg: String| CoreError::ProcessFailed(self.meta.name.clone(), msg);

        if self.path.is_empty() {
            return Err(fail("Geen bestandspad ingesteld".into()));
        }

        // Laad het beeld via de `image` crate en converteer naar RGBA f32.
        let img = image::open(&self.path)
            .map_err(|e| fail(format!("Kan '{}' niet openen: {}", self.path, e)))?
            .to_rgba32f();

        let (w, h) = img.dimensions();
        // `into_raw()` geeft een `Vec<f32>` in RGBA volgorde — precies wat PixelBuffer verwacht.
        let data: Vec<f32> = img.into_raw();

        let buf = PixelBuffer::from_vec(w, h, PixelFormat::LinearRgba, data)
            .map_err(|e| fail(e.to_string()))?;

        Ok(Arc::new(buf))
    }
}
