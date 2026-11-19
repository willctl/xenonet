use std::fmt;
use std::str::FromStr;

use crate::error::{invalid, ModelError};

fn is_lower_alnum(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

macro_rules! string_newtype {
    ($(#[$doc:meta])* $name:ident, $what:literal, $check:expr) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The name as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ModelError;

            fn from_str(text: &str) -> Result<Self, ModelError> {
                let check: fn(&str) -> Result<(), &'static str> = $check;
                check(text).map_err(|rule| invalid($what, rule))?;
                Ok(Self(text.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = ModelError;

            fn try_from(text: String) -> Result<Self, ModelError> {
                text.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({:?})"), self.0)
            }
        }
    };
}

string_newtype!(
    /// An opaque label, `wall_` and 1 to 64 of a-z, 0-9, `_`, `-`. Adytum
    /// compares walls and never reads one.
    Wall,
    "wall",
    |t| {
        let rest = t.strip_prefix("wall_").ok_or("it starts with wall_")?;
        if rest.is_empty() || rest.len() > 64 {
            return Err("1 to 64 characters after wall_");
        }
        if !rest.chars().all(|c| is_lower_alnum(c) || c == '_' || c == '-') {
            return Err("only a-z, 0-9, _ and - after wall_");
        }
        Ok(())
    }
);

string_newtype!(
    /// What a driver calls a VM or container: `adytum-` and 1 to 47 of a-z,
    /// 0-9, `-`, starting with a letter or digit and not ending in a dash.
    /// Anything else is not a cell Adytum made.
    CellName,
    "cell name",
    |t| {
        let rest = t.strip_prefix("adytum-").ok_or("it starts with adytum-")?;
        if rest.is_empty() || rest.len() > 47 {
            return Err("1 to 47 characters after adytum-");
        }
        if !rest.chars().all(|c| is_lower_alnum(c) || c == '-') {
            return Err("only a-z, 0-9 and - after adytum-");
        }
        if !rest.starts_with(is_lower_alnum) {
            return Err("a letter or digit right after adytum-");
        }
        if rest.ends_with('-') {
            return Err("no dash at the end");
        }
        Ok(())
    }
);

string_newtype!(
    /// A network's name: 1 to 48 of a-z, 0-9, `-`.
    NetworkName,
    "network name",
    |t| {
        if t.is_empty() || t.len() > 48 {
            return Err("1 to 48 characters");
        }
        if !t.chars().all(|c| is_lower_alnum(c) || c == '-') {
            return Err("only a-z, 0-9 and -");
        }
        Ok(())
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_accepts_labels_and_refuses_the_rest() {
        assert!("wall_a".parse::<Wall>().is_ok());
        assert!("wall_Team-1_x".parse::<Wall>().is_err());
        assert!("wall_".parse::<Wall>().is_err());
        assert!("a".parse::<Wall>().is_err());
        assert!(format!("wall_{}", "a".repeat(64)).parse::<Wall>().is_ok());
        assert!(format!("wall_{}", "a".repeat(65)).parse::<Wall>().is_err());
        assert!("wall_a b".parse::<Wall>().is_err());
    }

    #[test]
    fn cell_name_matches_adytum_pattern() {
        for ok in ["adytum-a", "adytum-test-1", "adytum-0x", &format!("adytum-{}", "a".repeat(47))] {
            assert!(ok.parse::<CellName>().is_ok(), "{ok:?}");
        }
        for bad in ["pane", "xenonet-vm1", "default", "adytum-", "adytum--a", "adytum-a-", "adytum-A", "adytum-a_b", &format!("adytum-{}", "a".repeat(48))] {
            assert!(bad.parse::<CellName>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn network_name_pattern() {
        assert!("net-1".parse::<NetworkName>().is_ok());
        assert!("".parse::<NetworkName>().is_err());
        assert!("Net".parse::<NetworkName>().is_err());
        assert!("a".repeat(49).parse::<NetworkName>().is_err());
    }

    #[test]
    fn json_is_the_plain_string_and_is_validated() {
        let n: CellName = serde_json::from_str("\"adytum-a\"").unwrap();
        assert_eq!(serde_json::to_string(&n).unwrap(), "\"adytum-a\"");
        assert!(serde_json::from_str::<CellName>("\"pane\"").is_err());
    }
}
