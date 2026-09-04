use std::fmt;

pub type KbResult<T> = Result<T, KbError>;

#[derive(Debug)]
pub enum KbError {
    NotFound(String),
    Invalid(String),
    Io(std::io::Error),
    #[cfg(feature = "index")]
    Db(rusqlite::Error),
    Embedding(String),
    Cancelled(String),
}

impl fmt::Display for KbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KbError::NotFound(what) => write!(f, "not found: {what}"),
            KbError::Invalid(what) => write!(f, "invalid: {what}"),
            KbError::Io(e) => write!(f, "io: {e}"),
            #[cfg(feature = "index")]
            KbError::Db(e) => write!(f, "db: {e}"),
            KbError::Embedding(what) => write!(f, "embedding: {what}"),
            KbError::Cancelled(what) => write!(f, "cancelled: {what}"),
        }
    }
}

impl std::error::Error for KbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KbError::Io(e) => Some(e),
            #[cfg(feature = "index")]
            KbError::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for KbError {
    fn from(e: std::io::Error) -> Self {
        KbError::Io(e)
    }
}

#[cfg(feature = "index")]
impl From<rusqlite::Error> for KbError {
    fn from(e: rusqlite::Error) -> Self {
        KbError::Db(e)
    }
}

impl KbError {
    pub(crate) fn invalid(what: impl Into<String>) -> Self {
        KbError::Invalid(what.into())
    }

    pub(crate) fn not_found(what: impl Into<String>) -> Self {
        KbError::NotFound(what.into())
    }
}
