//! Reader error type (split from design/claude-v04/reader_backend.rs `pub mod error`).

use serde::Serialize;

/// Serialises as {"code":"corrupt","detail":"..."} - JS gets a real object from invoke() rejections.
#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "detail", rename_all = "snake_case")]
pub enum ReaderError {
    #[error("book is not open")]
    BookNotOpen,
    #[error("source file missing: {0}")]
    SourceMissing(String),
    #[error("page out of range")]
    PageOutOfRange,
    #[error("corrupt or unreadable: {0}")]
    Corrupt(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("cancelled")]
    Cancelled,
    #[error("io: {0}")]
    Io(String),
    #[error("db: {0}")]
    Db(String),
}

impl ReaderError {
    pub fn http_status(&self) -> u16 {
        use ReaderError::*;
        match self {
            BookNotOpen | PageOutOfRange => 404,
            SourceMissing(_) => 410,
            BadRequest(_) => 400,
            Corrupt(_) | Unsupported(_) => 422,
            Cancelled => 503, // JS retries 503 (see ImageWindow.onError)
            Io(_) | Db(_) => 500,
        }
    }
    pub fn code(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v["code"].as_str().map(String::from))
            .unwrap_or_default()
    }
}

impl From<std::io::Error> for ReaderError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            Self::SourceMissing(e.to_string())
        } else {
            Self::Io(e.to_string())
        }
    }
}
impl From<rusqlite::Error> for ReaderError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}
impl From<zip::result::ZipError> for ReaderError {
    fn from(e: zip::result::ZipError) -> Self {
        use zip::result::ZipError as Z;
        match e {
            Z::Io(io) => io.into(),
            Z::UnsupportedArchive(m) => Self::Unsupported(m.to_string()),
            o => Self::Corrupt(o.to_string()),
        }
    }
}
impl From<image::ImageError> for ReaderError {
    fn from(e: image::ImageError) -> Self {
        Self::Corrupt(e.to_string())
    }
}

impl From<String> for ReaderError {
    fn from(e: String) -> Self {
        Self::Io(e)
    }
}
impl From<&str> for ReaderError {
    fn from(e: &str) -> Self {
        Self::Io(e.to_string())
    }
}
