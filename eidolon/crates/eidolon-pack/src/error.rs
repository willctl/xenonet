use eidolon_vault::VaultError;
use std::fmt;

/// Why packing, sealing, loading or unpacking failed.
#[derive(Debug)]
pub enum PackError {
    /// The vault or the keys refused: a chunk is missing, changed or
    /// unreadable.
    Vault(VaultError),
    /// A file operation failed.
    Io { what: &'static str, source: std::io::Error },
    /// A manifest that is not in the format, or does not match its files.
    BadManifest(String),
    /// An entry path that could write outside the folder.
    UnsafePath(String),
    /// The folder to unpack into already holds something.
    NotEmpty(String),
    /// The signer could not sign.
    Sign(String),
    /// The system gave no random numbers.
    Random,
}

impl PackError {
    pub(crate) fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> PackError {
        move |source| PackError::Io { what, source }
    }
}

impl From<VaultError> for PackError {
    fn from(e: VaultError) -> PackError {
        PackError::Vault(e)
    }
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackError::Vault(e) => write!(f, "{e}"),
            PackError::Io { what, source } => write!(f, "Could not {what}: {source}"),
            PackError::BadManifest(why) => write!(f, "The manifest is not usable: {why}. Fetch it again from the machine that has it."),
            PackError::UnsafePath(path) => write!(f, "The entry {path:?} would write outside its folder, so nothing was unpacked. Do not open this manifest."),
            PackError::NotEmpty(path) => write!(f, "{path} is not empty. Unpack into a folder that does not exist yet or is empty."),
            PackError::Sign(why) => write!(f, "Signing failed ({why}). Check the machine's key."),
            PackError::Random => f.write_str("The system gave no random numbers. Try again."),
        }
    }
}

impl std::error::Error for PackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PackError::Vault(e) => Some(e),
            PackError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, PackError>;
