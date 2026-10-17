//! core/demo-root.js: a Windows terminal or watcher can still hold the old
//! made-up workspace during a reload. Then that folder is left alone and
//! another made-up world is built beside it.

/// How the old workspace is taken out: recursively, forcing, with four
/// retries 50 ms apart.
pub const REMOVE_RETRIES: u32 = 4;
pub const REMOVE_RETRY_DELAY_MS: u32 = 50;

/// Whether a failure to take the old workspace out means building a fresh
/// one beside it (on Windows, when the folder is held open), rather than
/// passing the error on.
pub fn use_sibling(platform: &str, code: Option<&str>) -> bool {
    platform == "win32" && matches!(code, Some("EPERM" | "EBUSY" | "ENOTEMPTY"))
}

/// The prefix the fresh sibling's temporary folder is made from.
pub fn sibling_prefix(root: &str) -> String {
    format!("{root}-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_held_windows_folder_moves_aside() {
        assert!(use_sibling("win32", Some("EPERM")));
        assert!(use_sibling("win32", Some("EBUSY")));
        assert!(!use_sibling("win32", Some("EIO")));
        assert!(!use_sibling("linux", Some("EPERM")));
        assert_eq!(sibling_prefix("/t/demo"), "/t/demo-");
    }
}
