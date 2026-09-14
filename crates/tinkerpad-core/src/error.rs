//! Errors as the UI sees them.
//!
//! Commands never return a stringified error chain. They return a
//! [`CommandError`] with a stable machine-readable [`ErrorCode`] and a message
//! meant for a person.

use serde::Serialize;

/// Stable error categories the UI can branch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// A program or toolchain the command needed isn't installed.
    NotFound,
    /// The request itself was invalid.
    InvalidInput,
    /// Reading or writing a file failed.
    Io,
    /// Anything else; the message has the details.
    Internal,
}

/// The error type every Tauri command returns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let e = CommandError::new(ErrorCode::NotFound, "node is not installed");
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"code":"notFound","message":"node is not installed"}"#
        );
    }
}
