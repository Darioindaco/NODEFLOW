//! High-performance pixel buffer for float32 compositing.
//!
//! ## Memory layout
//!
//! Pixels are stored as **interleaved RGBA** – each sample is a `f32`.
//! For a 4K (3840 × 2160) image:
//!
//! ```text
//!   total floats = 3840 × 2160 × 4 = 33_177_600
//!   total bytes  = 33_177_600 × 4  = 132 710 400  ≈ 127 MiB
//! ```
//!
//! The backing allocation is over-aligned to **64 bytes** (cache-line size)
//! so that SIMD loads starting at any row boundary are always aligned.
//!
//! ## Thread-safety
//!
//! `PixelBuffer` is `Send + Sync`.  Mutable operations take `&mut self`.
//! Wrap in `Arc<Mutex<…>>` or `Arc<RwLock<…>>` for shared access.

use std::alloc::{alloc_zeroed, dealloc, Layout};

use serde::{Deserialize, Serialize};

use crate::error::BufferError;

/// The colour-space / channel interpretation of a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    /// Linear-light RGBA, each channel `f32`.
    LinearRgba,
    /// Premultiplied linear RGBA (`A·R`, `A·G`, `A·B`, `A`).
    PremultLinearRgba,
}

/// An axis-aligned rectangular region within a buffer.
#[derive(Debug, Clone, Copy)]
pub struct BufferRegion {
    pub x: u32, pub y: u32,
    pub w: u32, pub h: u32,
}

/// A 64-byte-aligned, heap-allocated buffer of `f32` pixels in RGBA order.
///
/// # Safety invariant
/// `self.ptr` points to a valid allocation of exactly `self.len() * 4` bytes,
/// aligned to `ALIGN`.
pub struct PixelBuffer {
    ptr:    *mut f32,
    width:  u32,
    height: u32,
    format: PixelFormat,
}

const ALIGN: usize = 64; // One cache line.

impl PixelBuffer {
    // ── Construction ─────────────────────────────────────────────────────────

    /// Allocate a zero-initialised buffer.
    pub fn new(
        width:  u32,
        height: u32,
        format: PixelFormat,
    ) -> Result<Self, BufferError> {
        let len   = (width as usize) * (height as usize) * 4; // floats
        let bytes = len * std::mem::size_of::<f32>();
        let layout = Layout::from_size_align(bytes, ALIGN)
            .expect("layout construction failed");

        // SAFETY: layout is non-zero because we bail on empty dimensions.
        let ptr = if len == 0 {
            std::ptr::NonNull::dangling().as_ptr()
        } else {
            let raw = unsafe { alloc_zeroed(layout) } as *mut f32;
            if raw.is_null() {
                return Err(BufferError::AllocationFailed { bytes });
            }
            raw
        };

        Ok(Self { ptr, width, height, format })
    }

    /// Wrap an existing `Vec<f32>` (copied into aligned storage).
    pub fn from_vec(
        width:  u32,
        height: u32,
        format: PixelFormat,
        data:   Vec<f32>,
    ) -> Result<Self, BufferError> {
        let mut buf = Self::new(width, height, format)?;
        let expected = buf.len();
        if data.len() != expected {
            return Err(BufferError::DimensionMismatch {
                expected_w: width, expected_h: height,
                got_w: (data.len() / 4) as u32, got_h: 1,
            });
        }
        buf.as_slice_mut().copy_from_slice(&data);
        Ok(buf)
    }

    /// Deep copy.
    pub fn clone_buffer(&self) -> Result<Self, BufferError> {
        let mut out = Self::new(self.width, self.height, self.format)?;
        out.as_slice_mut().copy_from_slice(self.as_slice());
        Ok(out)
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn width(&self)  -> u32 { self.width  }
    pub fn height(&self) -> u32 { self.height }
    pub fn format(&self) -> PixelFormat { self.format }

    /// Total number of `f32` values (w × h × 4).
    #[inline] pub fn len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 4
    }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Immutable flat slice over all RGBA floats.
    ///
    /// # Safety
    /// The pointer was allocated with `ALIGN` and this struct is the sole owner.
    #[inline]
    pub fn as_slice(&self) -> &[f32] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len()) }
    }

    /// Mutable flat slice.
    #[inline]
    pub fn as_slice_mut(&mut self) -> &mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len()) }
    }

    /// Return the raw pointer (for SIMD / FFI use).
    ///
    /// # Safety
    /// Caller must not outlive the buffer or violate aliasing rules.
    #[inline] pub unsafe fn as_ptr(&self)     -> *const f32 { self.ptr }
    #[inline] pub unsafe fn as_ptr_mut(&mut self) -> *mut f32   { self.ptr }

    // ── Per-pixel access ──────────────────────────────────────────────────────

    /// Return a `[R, G, B, A]` array for pixel `(x, y)`.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> [f32; 4] {
        let base = ((y * self.width + x) * 4) as usize;
        let s    = self.as_slice();
        [s[base], s[base+1], s[base+2], s[base+3]]
    }

    /// Write a `[R, G, B, A]` value to pixel `(x, y)`.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [f32; 4]) {
        let base = ((y * self.width + x) * 4) as usize;
        let s    = self.as_slice_mut();
        s[base] = rgba[0]; s[base+1] = rgba[1];
        s[base+2] = rgba[2]; s[base+3] = rgba[3];
    }

    // ── Region helpers ────────────────────────────────────────────────────────

    /// Validate that a [`BufferRegion`] fits inside this buffer.
    pub fn check_region(&self, r: &BufferRegion) -> Result<(), BufferError> {
        if r.x + r.w > self.width || r.y + r.h > self.height {
            return Err(BufferError::RegionOutOfBounds {
                x: r.x, y: r.y, w: r.w, h: r.h,
                bw: self.width, bh: self.height,
            });
        }
        Ok(())
    }

    /// Assert both buffers have identical dimensions.
    pub fn check_same_size(&self, other: &PixelBuffer) -> Result<(), BufferError> {
        if self.width != other.width || self.height != other.height {
            return Err(BufferError::DimensionMismatch {
                expected_w: self.width, expected_h: self.height,
                got_w: other.width,    got_h: other.height,
            });
        }
        Ok(())
    }
}

// ── Safety: raw-pointer ownership is unique ───────────────────────────────────
// SAFETY: PixelBuffer owns its allocation exclusively.
unsafe impl Send for PixelBuffer {}
unsafe impl Sync for PixelBuffer {}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        let len   = self.len();
        let bytes = len * std::mem::size_of::<f32>();
        if len > 0 {
            let layout = Layout::from_size_align(bytes, ALIGN).unwrap();
            unsafe { dealloc(self.ptr as *mut u8, layout) };
        }
    }
}

impl std::fmt::Debug for PixelBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PixelBuffer({}×{} {:?})", self.width, self.height, self.format)
    }
}
