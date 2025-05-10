use crate::types::TunnelMessageType;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TunnelError {
    #[error("no auth token provided")]
    NoAuthTokenProvided,
    #[error("stored token is empty")]
    EmptyToken,
    #[error("invalid local port: {0}")]
    InvalidLocalPort(String),
    #[error("authentication failed: {0}")]
    AuthFailure(String),
    #[error("tunnel connection closed by peer or EOF")]
    ConnectionClosed,
    #[error("tunnel connection timed out")]
    TunnelTimeout,
    #[error("operation timed out: {0}")]
    OperationTimeout(String),
    #[error("duplicate port: {0}")]
    DuplicatePort(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("network dialing error: {0}")]
    DialError(String),
    #[error("server returned unexpected message type: {0:?}")]
    UnexpectedMessageType(TunnelMessageType),
    #[error("server did not send TunnelCreated message after successful authentication")]
    TunnelNotCreatedAfterAuth,
    #[error("missing expected header in server response: {0}")]
    MissingHeader(String),
    #[error("failed to parse integer from header: {0}")]
    HeaderParseIntError(#[from] std::num::ParseIntError),
    #[error("internal error: {0}")]
    InternalError(String),
    #[error("SDK config is required (e.g. auth token)")]
    SdkConfigMissing,
    #[error("connection not established or already closed")]
    NotConnected,
    #[error("failed to join task: {0}")]
    TaskJoinError(#[from] tokio::task::JoinError),
}
