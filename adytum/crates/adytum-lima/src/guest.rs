use adytum_driver::DriverError;

/// A path inside a cell: absolute, no `..`, no control characters.
pub fn check_guest_path(path: &str) -> Result<(), DriverError> {
    let bad = |rule: &'static str| DriverError::BadPath { path: path.to_owned(), rule };
    if !path.starts_with('/') {
        return Err(bad("a guest path must be absolute"));
    }
    if path.chars().any(char::is_control) {
        return Err(bad("it has a control character"));
    }
    if path.contains(':') {
        return Err(bad("limactl reads a colon as the end of an instance name"));
    }
    if path.split('/').any(|part| part == "..") {
        return Err(bad("it climbs out with .."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_paths_are_absolute_and_stay_put() {
        for ok in ["/", "/tmp/a b", "/tmp/..hidden", "/a/b.c"] {
            assert!(check_guest_path(ok).is_ok(), "{ok}");
        }
        for bad in ["tmp/a", "", "/tmp/../etc", "/..", "/a/..", "/a\nb", "/a:b", "-r"] {
            assert!(check_guest_path(bad).is_err(), "{bad:?}");
        }
    }
}
