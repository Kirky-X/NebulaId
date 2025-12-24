use derive_more::Display;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Display, Serialize, Deserialize, Clone)]
pub enum CoreError {
    #[display("Invalid ID format: {}", _0)]
    InvalidIdFormat(String),

    #[display("Invalid ID string: {}", _0)]
    InvalidIdString(String),

    #[display("Invalid algorithm type: {}", _0)]
    InvalidAlgorithmType(String),

    #[display("Clock moved backward, last timestamp: {}", last_timestamp)]
    ClockMovedBackward { last_timestamp: u64 },

    #[display("Sequence overflow, timestamp: {}", timestamp)]
    SequenceOverflow { timestamp: u64 },

    #[display("Segment exhausted, max_id: {}", max_id)]
    SegmentExhausted { max_id: u64 },

    #[display("Database error: {}", _0)]
    DatabaseError(String),

    #[display("Cache error: {}", _0)]
    CacheError(String),

    #[display("Configuration error: {}", _0)]
    ConfigurationError(String),

    #[display("Authentication error: {}", _0)]
    AuthenticationError(String),

    #[display("Rate limit exceeded")]
    RateLimitExceeded,

    #[display("Resource not found: {}", _0)]
    NotFound(String),

    #[display("Workspace disabled: {}", _0)]
    WorkspaceDisabled(String),

    #[display("Biz tag not found: {}", _0)]
    BizTagNotFound(String),

    #[display("API key disabled")]
    ApiKeyDisabled,

    #[display("API key expired")]
    ApiKeyExpired,

    #[display("Invalid API key signature")]
    InvalidApiKeySignature,

    #[display("Etcd error: {}", _0)]
    EtcdError(String),

    #[display("Parse error: {}", _0)]
    ParseError(String),

    #[display("I/O error: {}", _0)]
    IoError(String),

    #[display("Timeout error")]
    TimeoutError,

    #[display("Internal error: {}", _0)]
    InternalError(String),

    #[display("Unknown error")]
    #[from(ignore)]
    Unknown,
}

impl From<std::num::ParseIntError> for CoreError {
    fn from(e: std::num::ParseIntError) -> Self {
        CoreError::ParseError(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::IoError(e.to_string())
    }
}

impl From<uuid::Error> for CoreError {
    fn from(e: uuid::Error) -> Self {
        CoreError::ParseError(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: i32,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl ErrorResponse {
    pub fn new(code: i32, message: String) -> Self {
        Self {
            code,
            message,
            details: None,
        }
    }

    pub fn with_details(code: i32, message: String, details: serde_json::Value) -> Self {
        Self {
            code,
            message,
            details: Some(details),
        }
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;

pub const ERROR_CODE_INVALID_REQUEST: i32 = 400;
pub const ERROR_CODE_UNAUTHORIZED: i32 = 401;
pub const ERROR_CODE_FORBIDDEN: i32 = 403;
pub const ERROR_CODE_NOT_FOUND: i32 = 404;
pub const ERROR_CODE_RATE_LIMIT: i32 = 429;
pub const ERROR_CODE_INTERNAL_ERROR: i32 = 500;
pub const ERROR_CODE_SERVICE_UNAVAILABLE: i32 = 503;
