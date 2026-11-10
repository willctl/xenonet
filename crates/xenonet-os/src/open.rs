//! Opening a URL, a file or a folder on this machine: the program each
//! system opens them with, and the editor's link to a folder.

use regex::Regex;
use std::sync::OnceLock;

fn drive() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z]:/").expect("a fixed pattern"))
}

/// vscode://file/C:/x on Windows, vscode://file/Users/x on macOS and
/// Linux. POSIX paths are encoded a segment at a time, because macOS's
/// `open` refuses a URL with a space in it; Explorer takes Windows paths
/// as they are.
pub fn vscode_url(folder: &str) -> String {
    let p = folder.replace('\\', "/");
    if drive().is_match(&p) {
        return format!("vscode://file/{p}");
    }
    format!("vscode://file/{}", p.trim_start_matches('/').split('/').map(xenonet_text::url::component).collect::<Vec<_>>().join("/"))
}

/// The program that opens a URL or a file on `platform`, as [program, args].
pub fn launch_command(target: &str, platform: &str) -> (String, Vec<String>) {
    let cmd = match platform {
        "win32" => "explorer.exe",
        "darwin" => "open",
        _ => "xdg-open",
    };
    (cmd.into(), vec![target.into()])
}

/// A file shown in the file manager: selected in Finder or Explorer, its
/// folder opened elsewhere.
pub fn reveal_command(file: &str, platform: &str) -> (String, Vec<String>) {
    match platform {
        "darwin" => ("open".into(), vec!["-R".into(), file.into()]),
        "win32" => ("explorer.exe".into(), vec![format!("/select,{file}")]),
        _ => {
            // The folder: everything before the last separator, or "/".
            let cut = file.rfind(['/', '\\']).map_or(file, |i| &file[..i]);
            ("xdg-open".into(), vec![if cut.is_empty() { "/".into() } else { cut.into() }])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_become_editor_links_and_files_are_revealed() {
        assert_eq!(vscode_url("/Users/me/Projects/a b#2"), "vscode://file/Users/me/Projects/a%20b%232");
        assert_eq!(vscode_url("C:\\Users\\me"), "vscode://file/C:/Users/me");
        assert_eq!(reveal_command("/home/w/x.json", "linux").1, ["/home/w"]);
        assert_eq!(reveal_command("x.json", "linux").1, ["x.json"]);
        assert_eq!(reveal_command("/x.json", "linux").1, ["/"]);
    }
}
