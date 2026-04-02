use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    NotFound(String),
    Validation(String),
    Adapter(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::Validation(message) => write!(f, "validation error: {message}"),
            Self::Adapter(message) => write!(f, "adapter error: {message}"),
        }
    }
}

impl Error for AppError {}
