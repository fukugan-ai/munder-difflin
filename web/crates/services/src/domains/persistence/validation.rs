use super::RepositoryError;

pub(super) fn validate_segment(
    value: &str,
    maximum_bytes: usize,
    message: &'static str,
) -> Result<(), RepositoryError> {
    if value.is_empty() || value.len() > maximum_bytes {
        return Err(RepositoryError::InvalidInput(message));
    }
    Ok(())
}

pub(super) fn validate_payload(payload: &str) -> Result<(), RepositoryError> {
    if payload.trim().is_empty() {
        return Err(RepositoryError::InvalidInput("JSON payload is required"));
    }
    Ok(())
}

pub(super) fn clamp_limit(limit: u16, maximum: u16) -> i64 {
    i64::from(limit.clamp(1, maximum))
}

pub(super) fn u64_to_i64(value: u64, message: &'static str) -> Result<i64, RepositoryError> {
    i64::try_from(value).map_err(|_| RepositoryError::InvalidInput(message))
}

pub(super) fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{clamp_limit, escape_like, u64_to_i64, validate_payload, validate_segment};

    #[test]
    fn segment_rejects_empty_value() {
        assert!(validate_segment("", 8, "invalid").is_err());
    }

    #[test]
    fn segment_accepts_maximum_length() {
        assert!(validate_segment("12345678", 8, "invalid").is_ok());
    }

    #[test]
    fn payload_rejects_whitespace() {
        assert!(validate_payload("  \n").is_err());
    }

    #[test]
    fn zero_limit_becomes_one() {
        assert_eq!(clamp_limit(0, 500), 1);
    }

    #[test]
    fn excessive_limit_is_bounded() {
        assert_eq!(clamp_limit(u16::MAX, 500), 500);
    }

    #[test]
    fn maximum_database_integer_is_accepted() {
        assert!(matches!(
            u64_to_i64(i64::MAX as u64, "overflow"),
            Ok(i64::MAX)
        ));
    }

    #[test]
    fn integer_overflow_is_rejected() {
        assert!(u64_to_i64(u64::MAX, "overflow").is_err());
    }

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(escape_like(r"100%_done\ok"), r"100\%\_done\\ok");
    }
}
