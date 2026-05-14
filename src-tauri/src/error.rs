use thiserror::Error;

#[derive(Error, Debug)]
pub enum QuikFindError {
    #[error("Index not initialized")]
    IndexNotReady,

    #[error("Searcher not available")]
    SearcherNotReady,

    #[error("Index writer not available")]
    WriterNotReady,

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Tantivy error: {0}")]
    Tantivy(#[from] tantivy::error::TantivyError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Settings error: {0}")]
    Settings(String),

    #[error("Path not found: {0}")]
    PathNotFound(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("{0}")]
    Generic(String),
}

impl From<QuikFindError> for String {
    fn from(err: QuikFindError) -> Self {
        err.to_string()
    }
}

impl From<&str> for QuikFindError {
    fn from(s: &str) -> Self {
        Self::Generic(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, QuikFindError>;
