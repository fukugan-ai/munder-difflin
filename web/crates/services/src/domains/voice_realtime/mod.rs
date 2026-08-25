#![forbid(unsafe_code)]

mod action_policy;
mod completion;
mod floor;
mod freeflow;
mod pricing;
mod secrets;
mod upstream;

pub use action_policy::{ActionDisposition, ActionPolicy, ConfirmationOutcome, VoicePolicyError};
pub use completion::{CompletionDetector, CompletionInboxMessage, CompletionTask, PendingDispatch};
pub use floor::{FloorAgent, FloorObserver, FloorPty, FloorTask};
pub use freeflow::{AudioValidationError, ValidatedAudio, validate_audio};
pub use pricing::{RealtimeCostMeter, compute_realtime_usd};
pub use secrets::{SecretReader, SecretSlot};
pub use upstream::{VoiceUpstreamClient, VoiceUpstreamError};
