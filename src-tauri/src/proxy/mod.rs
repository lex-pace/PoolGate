pub mod auth;
pub mod concurrency;
pub mod health;
pub mod logger;
pub mod protocol;
pub mod router;
pub mod runtime;
pub mod server;
pub mod stream;

/// Extracted error detail from an upstream (provider) error body. Protocol
/// handlers attach this to the response so route logs can record the
/// downstream-provided message instead of a bare HTTP status.
#[derive(Clone, Debug)]
pub struct UpstreamErrorDetail(pub String);
