use std::fmt::{Display, Formatter};

/// Concrete persistence boundary failures. Database details remain available as
/// a source for server-side diagnostics but are not included in Display output.
#[derive(Debug)]
pub enum RepositoryError {
    InvalidInput(&'static str),
    Conflict,
    NotFound,
    SequenceExhausted,
    InvalidData(&'static str),
    Database(sqlx::Error),
}

impl Display for RepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput(message) => message,
            Self::Conflict => "persistence revision conflict",
            Self::NotFound => "persistence record not found",
            Self::SequenceExhausted => "persistence sequence exhausted",
            Self::InvalidData(message) => message,
            Self::Database(_) => "PostgreSQL persistence operation failed",
        })
    }
}

impl std::error::Error for RepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for RepositoryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::RepositoryError;

    #[test]
    fn database_display_does_not_expose_driver_detail() {
        let error = RepositoryError::Database(sqlx::Error::RowNotFound);

        assert_eq!(error.to_string(), "PostgreSQL persistence operation failed");
    }

    #[test]
    fn conflict_has_stable_message() {
        assert_eq!(
            RepositoryError::Conflict.to_string(),
            "persistence revision conflict"
        );
    }
}
