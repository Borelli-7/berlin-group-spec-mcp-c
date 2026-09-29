use thiserror::Error;

/// Structured error type shared by all services and adapters.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid locator '{locator}': {reason}")]
    InvalidLocator { locator: String, reason: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("requirements mapping error: {0}")]
    Requirements(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("search index error: {0}")]
    Search(String),
    #[error("integrity error: {0}")]
    Integrity(String),
    #[error("i/o error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

impl CoreError {
    /// Stable machine-readable error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidLocator { .. } => "invalid_locator",
            Self::NotFound(_) => "not_found",
            Self::Config(_) => "config",
            Self::Manifest(_) => "manifest",
            Self::Requirements(_) => "requirements",
            Self::Storage(_) => "storage",
            Self::Search(_) => "search",
            Self::Integrity(_) => "integrity",
            Self::Io { .. } => "io",
        }
    }

    /// True when the error was caused by caller input rather than by the system.
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidInput(_) | Self::InvalidLocator { .. } | Self::NotFound(_)
        )
    }

    pub fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T, E = CoreError> = std::result::Result<T, E>;
