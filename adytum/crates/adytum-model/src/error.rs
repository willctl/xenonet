use std::fmt;

/// Why a value could not be made: a string that is not a valid name, id or
/// pin, or the system had no random bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// The text is not a valid value of the named kind.
    Invalid {
        /// What was being parsed, such as "cell name".
        what: &'static str,
        /// The rule it broke, as a sentence.
        rule: &'static str,
    },
    /// The system could not give random bytes for a new id.
    Random,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { what, rule } => write!(f, "Not a valid {what}: {rule}"),
            Self::Random => f.write_str("The system could not give random bytes for a new id. Try again, and check that the machine's random source works."),
        }
    }
}

impl std::error::Error for ModelError {}

pub(crate) fn invalid(what: &'static str, rule: &'static str) -> ModelError {
    ModelError::Invalid { what, rule }
}
