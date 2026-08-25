#![forbid(unsafe_code)]

use md_web_contracts::domains::voice_realtime::{MAX_AUDIO_BYTES, TranscriptionMetadata};

const ALLOWED_MIME_TYPES: [&str; 4] = ["audio/webm", "audio/ogg", "audio/mp4", "audio/mpeg"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioValidationError {
    Empty,
    TooLarge,
    UnsupportedMime,
    LengthMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedAudio<'a> {
    pub bytes: &'a [u8],
    pub mime_type: &'a str,
    pub filename: &'a str,
    pub language: Option<&'a str>,
}

pub fn validate_audio<'a>(
    metadata: &'a TranscriptionMetadata,
    bytes: &'a [u8],
) -> Result<ValidatedAudio<'a>, AudioValidationError> {
    if bytes.is_empty() {
        return Err(AudioValidationError::Empty);
    }
    if metadata.byte_length > MAX_AUDIO_BYTES || bytes.len() as u64 > MAX_AUDIO_BYTES {
        return Err(AudioValidationError::TooLarge);
    }
    if metadata.byte_length != bytes.len() as u64 {
        return Err(AudioValidationError::LengthMismatch);
    }
    let mime_type = metadata
        .mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if !ALLOWED_MIME_TYPES.contains(&mime_type) {
        return Err(AudioValidationError::UnsupportedMime);
    }
    Ok(ValidatedAudio {
        bytes,
        mime_type,
        filename: metadata.filename.as_str(),
        language: metadata.language.as_deref(),
    })
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::voice_realtime::{MAX_AUDIO_BYTES, TranscriptionMetadata};

    use super::{AudioValidationError, validate_audio};

    fn metadata(byte_length: u64, mime_type: &str) -> TranscriptionMetadata {
        TranscriptionMetadata {
            byte_length,
            mime_type: String::from(mime_type),
            filename: String::from("dictation.webm"),
            language: None,
        }
    }

    #[test]
    fn empty_audio_is_rejected() {
        let meta = metadata(0, "audio/webm");
        let result = validate_audio(&meta, &[]);

        assert_eq!(result, Err(AudioValidationError::Empty));
    }

    #[test]
    fn unsupported_mime_is_rejected() {
        let meta = metadata(1, "text/plain");
        let result = validate_audio(&meta, &[1]);

        assert_eq!(result, Err(AudioValidationError::UnsupportedMime));
    }

    #[test]
    fn declared_length_must_match() {
        let meta = metadata(2, "audio/webm");
        let result = validate_audio(&meta, &[1]);

        assert_eq!(result, Err(AudioValidationError::LengthMismatch));
    }

    #[test]
    fn maximum_plus_one_is_rejected() {
        let meta = metadata(MAX_AUDIO_BYTES + 1, "audio/webm");
        let result = validate_audio(&meta, &[1]);

        assert_eq!(result, Err(AudioValidationError::TooLarge));
    }

    #[test]
    fn valid_opus_webm_is_accepted() {
        let meta = metadata(2, "audio/webm;codecs=opus");
        let result = validate_audio(&meta, &[1, 2]);

        assert!(result.is_ok());
    }
}
