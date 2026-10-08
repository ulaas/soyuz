//! Error type shared by providers, credentials and tools.

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum AiError {
    #[error("network error: {0}")]
    Network(String),
    #[error("request timed out ({0})")]
    Timeout(String),
    #[error("rate limited{}", retry_after.map(|d| format!(" (retry after {}s)", d.as_secs())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },
    #[error("provider overloaded (HTTP {0})")]
    Overloaded(u16),
    #[error("authentication failed (HTTP {status}): {message}")]
    Auth { status: u16, message: String },
    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("provider error ({kind}): {message}")]
    Provider { kind: String, message: String },
    #[error("cannot decode provider response: {0}")]
    Decode(String),
    #[error("cancelled")]
    Cancelled,
    #[error("monthly budget reached for profile “{profile}”")]
    BudgetExceeded { profile: String },
    #[error("credential store: {0}")]
    Credential(String),
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("configuration: {0}")]
    Config(String),
}

impl AiError {
    /// Worth retrying (before any output was streamed).
    pub fn is_retryable(&self) -> bool {
        matches!(self, AiError::RateLimited { .. } | AiError::Overloaded(_) | AiError::Network(_))
    }
}

pub type AiResult<T> = Result<T, AiError>;
