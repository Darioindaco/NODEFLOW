use thiserror::Error;

#[derive(Debug, Error)]
pub enum OfxError {
    #[error("Failed to load OFX library '{path}': {reason}")]
    LibraryLoad { path: String, reason: String },

    #[error("OFX library missing symbol '{0}'")]
    MissingSymbol(String),

    #[error("OFX plugin #{index} returned status code {code}")]
    PluginStatus { index: usize, code: i32 },

    #[error("OFX host action '{action}' failed with status {code}")]
    ActionFailed { action: String, code: i32 },

    #[error("Plugin handle is null")]
    NullHandle,

    #[error("OFX property '{0}' not found")]
    PropertyNotFound(String),

    #[error("OFX type mismatch for property '{prop}': expected {expected}, got {got}")]
    TypeMismatch { prop: String, expected: String, got: String },

    #[error(transparent)]
    Core(#[from] nodeflow_core::CoreError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
