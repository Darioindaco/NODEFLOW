//! OpenFX (OFX) host and plugin-loader.
//!
//! # What is OpenFX?
//! OpenFX is an open C API standard (originally by The Foundry) that lets
//! visual-effects plugins run inside any compatible host (Nuke, Natron,
//! Resolve, …).  The ABI is defined in `ofxCore.h` / `ofxImageEffect.h`.
//!
//! # Design
//! ```text
//!  ┌──────────────────────────────────────────┐
//!  │           NodeFlow Host (Rust)           │
//!  │  ┌─────────────────────────────────────┐ │
//!  │  │  OfxHost  (implements OfxHost C API) │ │
//!  │  │  - provides Property / Param suites  │ │
//!  │  │  - provides ImageEffect suite        │ │
//!  │  └────────────────┬────────────────────┘ │
//!  │                   │ FFI calls             │
//!  │  ┌────────────────▼────────────────────┐ │
//!  │  │  OfxPluginLibrary                   │ │
//!  │  │  - loads .ofx shared lib via dlopen │ │
//!  │  │  - enumerates OfxPlugin entries     │ │
//!  │  │  - wraps each plugin as OfxNode     │ │
//!  │  └────────────────────────────────────┘ │
//!  └──────────────────────────────────────────┘
//! ```
//!
//! # Safety policy
//! All FFI calls go through named `unsafe` blocks with explicit SAFETY
//! comments.  The safe public API never exposes raw pointers.

pub mod ffi;
pub mod host;
pub mod plugin;
pub mod node_adapter;
pub mod error;

pub use host::OfxHost;
pub use plugin::OfxPluginLibrary;
pub use node_adapter::OfxNode;
pub use error::OfxError;
