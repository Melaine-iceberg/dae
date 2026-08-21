use serde::Serialize;
use specta::Type;
use std::io;
use thiserror::Error;

#[derive(Debug, Clone, Error, Serialize, Type)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum FileSystemError {
    #[error("The requested path was not found: {0}")]
    NotFound(String),
    #[error("Permission was denied: {0}")]
    PermissionDenied(String),
    #[error("The requested path is not a directory: {0}")]
    NotDirectory(String),
    #[error("The file system operation failed: {0}")]
    Io(String),
    #[error("An item already exists at the destination: {0}")]
    AlreadyExists(String),
    #[error("The requested operation has invalid input: {0}")]
    InvalidInput(String),
    #[error("The storage backend does not support this operation: {0}")]
    Unsupported(String),
    #[error("The directory operation could not complete: {0}")]
    Internal(String),
}

impl From<io::Error> for FileSystemError {
    fn from(error: io::Error) -> Self {
        let message = error.to_string();

        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound(message),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(message),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(message),
            io::ErrorKind::InvalidInput => Self::InvalidInput(message),
            _ => Self::Io(message),
        }
    }
}
