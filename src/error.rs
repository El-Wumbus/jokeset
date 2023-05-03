use thiserror::Error;
use tokio::io;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unhandled: {0}")]
    Unhandled(String),

    #[error("I/O: {0}")]
    Io(io::Error),

    #[error("Http(s) request: {0}")]
    Request(reqwest::Error),

    #[error("JSON: {0}")]
    Json(serde_json::Error),
}

impl From<tokio::io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Request(value)
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(value: tokio::task::JoinError) -> Self {
        Self::Unhandled(format!("JoinError: {value}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Error::Json(value)
    }
}