#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretSlot {
    Groq,
    OpenAi,
}

/// Server-only abstraction. Implementations must never expose plaintext through a DTO.
pub trait SecretReader {
    type Error;

    fn has_secret(&self, slot: SecretSlot) -> Result<bool, Self::Error>;
    fn with_secret<T>(
        &self,
        slot: SecretSlot,
        use_secret: impl FnOnce(&str) -> T,
    ) -> Result<Option<T>, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{SecretReader, SecretSlot};

    struct Missing;

    impl SecretReader for Missing {
        type Error = ();

        fn has_secret(&self, _slot: SecretSlot) -> Result<bool, Self::Error> {
            Ok(false)
        }

        fn with_secret<T>(
            &self,
            _slot: SecretSlot,
            _use_secret: impl FnOnce(&str) -> T,
        ) -> Result<Option<T>, Self::Error> {
            Ok(None)
        }
    }

    #[test]
    fn missing_reader_returns_presence_only() -> Result<(), ()> {
        assert!(!Missing.has_secret(SecretSlot::OpenAi)?);
        Ok(())
    }
}
