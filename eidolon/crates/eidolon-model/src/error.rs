use std::fmt;

/// Why a value could not be made or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// An id, wall, name or path that is not in its format.
    BadValue { what: &'static str, rule: &'static str },
    /// A key or signature that is not base64url of the right length.
    BadKey { what: &'static str },
    /// The system gave no random numbers.
    Random,
    /// The signer could not sign.
    Sign(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::BadValue { what, rule } => write!(f, "eidolon: that {what} is not valid. It must be {rule}."),
            ModelError::BadKey { what } => write!(f, "eidolon: that {what} is not base64url of the right length."),
            ModelError::Random => write!(f, "eidolon: the system gave no random numbers. Try again."),
            ModelError::Sign(why) => write!(f, "eidolon: signing failed ({why}). Check the machine's key."),
        }
    }
}

impl std::error::Error for ModelError {}
