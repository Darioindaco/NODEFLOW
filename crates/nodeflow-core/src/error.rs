use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Node '{0}' not found in DAG")]
    NodeNotFound(String),

    #[error("Adding edge from '{from}' → '{to}' would create a cycle")]
    CycleDetected { from: String, to: String },

    #[error("Node '{0}' has no registered input at port {1}")]
    InvalidInputPort(String, usize),

    #[error("Node '{0}' process() failed: {1}")]
    ProcessFailed(String, String),

    #[error("Scheduler error: {0}")]
    Scheduler(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
