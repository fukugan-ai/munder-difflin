use serde::{Deserialize, Serialize};

/// Stable machine-readable category for an API failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    NotFound,
    Conflict,
    ServiceUnavailable,
    Internal,
}

/// Error payload shared by the Dioxus client and server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message_ja: String,
    pub request_id: String,
}

#[cfg(test)]
mod tests {
    use super::{ApiError, ErrorCode};

    #[test]
    fn api_error_preserves_empty_message() {
        let error = ApiError {
            code: ErrorCode::Internal,
            message_ja: String::new(),
            request_id: String::from("request-1"),
        };

        assert!(error.message_ja.is_empty());
    }
}
