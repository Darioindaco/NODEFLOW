use thiserror::Error;

#[derive(Debug, Error)]
pub enum BufferError {
    #[error("Buffer dimensions mismatch: expected {expected_w}×{expected_h}, got {got_w}×{got_h}")]
    DimensionMismatch {
        expected_w: u32, expected_h: u32,
        got_w: u32,      got_h: u32,
    },

    #[error("Allocation of {bytes} bytes failed (OOM)")]
    AllocationFailed { bytes: usize },

    #[error("Region {x}+{w} × {y}+{h} exceeds buffer {bw}×{bh}")]
    RegionOutOfBounds { x: u32, y: u32, w: u32, h: u32, bw: u32, bh: u32 },
}
