//! Server-owned office projection and deterministic UI reducers.

mod projection;
mod toast;

pub use projection::{OfficeCommand, OfficeProjection, OfficeUiError};
pub use toast::CompletionToastStack;
