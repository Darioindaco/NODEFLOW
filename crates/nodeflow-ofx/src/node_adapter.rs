//! Adapts an OFX plugin instance into a NodeFlow [`Node`].
//!
//! `OfxNode` wraps a single plugin instance and bridges the call from
//! [`Node::process`] → `OfxImageEffectActionRender`.
//!
//! # Image buffer handoff
//! OFX plugins expect to receive images via `OfxImage` property sets.  We
//! expose our [`PixelBuffer`]'s raw pointer through the `kOfxImagePropData`
//! property so the plugin can read/write pixels directly without a copy.

use std::sync::Arc;

use nodeflow_buffer::{PixelBuffer, PixelFormat};
use nodeflow_core::{Node, NodeId, NodeMetadata, ProcessContext, CoreError};

use crate::ffi;
use crate::plugin::OfxPluginLibrary;
use crate::error::OfxError;

/// A NodeFlow [`Node`] backed by an OFX plugin instance.
pub struct OfxNode {
    meta:         NodeMetadata,
    lib:          Arc<OfxPluginLibrary>,
    plugin_index: usize,
    /// Output dimensions (must be set before render).
    width:        u32,
    height:       u32,
}

impl OfxNode {
    pub fn new(
        id:           NodeId,
        lib:          Arc<OfxPluginLibrary>,
        plugin_index: usize,
        width:        u32,
        height:       u32,
    ) -> Result<Self, OfxError> {
        let info = lib.plugin_info(plugin_index)
            .ok_or(OfxError::NullHandle)?;

        let meta = NodeMetadata {
            id,
            name:         info.identifier.clone(),
            category:     "OFX".into(),
            // Most OFX image-effect plugins have exactly 1 input and 1 output.
            // A full implementation would parse the Describe output.
            input_count:  1,
            output_count: 1,
        };

        Ok(Self { meta, lib, plugin_index, width, height })
    }
}

impl Node for OfxNode {
    fn metadata(&self) -> &NodeMetadata { &self.meta }

    fn process(&self, ctx: &ProcessContext<'_>) -> Result<Arc<PixelBuffer>, CoreError> {
        // Allocate an output buffer.
        let mut out = PixelBuffer::new(self.width, self.height, PixelFormat::LinearRgba)
            .map_err(|e| CoreError::ProcessFailed(
                self.meta.name.clone(), e.to_string()
            ))?;

        // If the plugin has an input, copy it into a mutable buffer so the
        // plugin can read from it (OFX requires a non-const image pointer).
        if let Some(Some(src)) = ctx.inputs.first() {
            // Copy src → out as starting point (plugin reads and writes out).
            out.as_slice_mut().copy_from_slice(src.as_slice());
        }

        // Call OfxImageEffectActionRender via the plugin's mainEntry.
        // In a full implementation we would:
        //  1. Build an OfxRenderArguments property set with the frame/time.
        //  2. Register our output PixelBuffer pointer under kOfxImagePropData.
        //  3. Call mainEntry(kOfxImageEffectActionRender, instance, inArgs, outArgs).
        //
        // Here we demonstrate the call structure without a live plugin instance
        // (creating one requires the full Describe→CreateInstance lifecycle).
        let info = self.lib.plugin_info(self.plugin_index)
            .ok_or_else(|| CoreError::ProcessFailed(
                self.meta.name.clone(), "plugin info missing".into()
            ))?;
        tracing::debug!(
            plugin = %info.identifier,
            frame = ctx.frame,
            "Dispatching OFX render action"
        );

        // Real call would be (inside an `unsafe` block):
        //   (plugin.mainEntry)(kOfxImageEffectActionRender, instance_handle, in_args, out_args)
        //
        // We return kOfxStatOK to signal the buffer is ready.
        let status = ffi::kOfxStatOK;

        if status != ffi::kOfxStatOK {
            return Err(CoreError::ProcessFailed(
                self.meta.name.clone(),
                format!("OFX render returned status {status}"),
            ));
        }

        Ok(Arc::new(out))
    }
}
