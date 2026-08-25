use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum DomainError {
    InvalidInput(&'static str),
    OutsideManagedRoot,
    NotFound,
    Unavailable(&'static str),
    Io(std::io::Error),
    Database(sqlx::Error),
    Serialization(serde_json::Error),
}

impl Display for DomainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "invalid input: {message}"),
            Self::OutsideManagedRoot => formatter.write_str("path is outside the managed root"),
            Self::NotFound => formatter.write_str("requested item was not found"),
            Self::Unavailable(message) => write!(formatter, "service unavailable: {message}"),
            Self::Io(error) => write!(formatter, "filesystem operation failed: {error}"),
            Self::Database(error) => write!(formatter, "database operation failed: {error}"),
            Self::Serialization(error) => write!(formatter, "data serialization failed: {error}"),
        }
    }
}

impl std::error::Error for DomainError {}

impl From<std::io::Error> for DomainError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<sqlx::Error> for DomainError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for DomainError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

#[cfg(test)]
mod tests {
    use super::DomainError;

    #[test]
    fn invalid_input_has_stable_operator_message() {
        assert_eq!(
            DomainError::InvalidInput("empty query").to_string(),
            "invalid input: empty query"
        );
    }
}
