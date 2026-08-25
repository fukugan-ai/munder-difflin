#![forbid(unsafe_code)]

mod error;
mod event;
mod health;

pub use error::{ApiError, ErrorCode};
pub use event::{AppEvent, EventEnvelope};
pub use health::{HealthSnapshot, PersistenceCode, PersistenceStatus};
