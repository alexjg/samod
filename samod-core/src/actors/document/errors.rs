use std::fmt;

/// Errors that can occur during document operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    /// Document is not yet ready for operations (not loaded).
    DocumentNotReady,
    InvalidState(String),
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentError::DocumentNotReady => {
                write!(f, "Document is not yet ready for operations")
            }
            DocumentError::InvalidState(msg) => {
                write!(f, "Invalid state: {msg}")
            }
        }
    }
}

impl std::error::Error for DocumentError {}
