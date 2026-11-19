use crate::error::{Result, ViewError};

/// The longest password VNC's classic authentication uses.
pub const SECRET_LEN: usize = 8;

const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Eight random letters and digits. Bytes of 248 and over are thrown away so
/// every character is equally likely.
pub fn make() -> Result<String> {
    let mut out = String::with_capacity(SECRET_LEN);
    while out.len() < SECRET_LEN {
        let mut buf = [0u8; 16];
        getrandom::fill(&mut buf).map_err(|e| ViewError::NoRandom(e.to_string()))?;
        for b in buf {
            if out.len() < SECRET_LEN && b < 248 {
                out.push(char::from(ALPHABET[usize::from(b) % ALPHABET.len()]));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_eight_printable_characters_and_differs_each_time() {
        let a = make().unwrap();
        let b = make().unwrap();
        assert_eq!(a.len(), SECRET_LEN);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }
}
