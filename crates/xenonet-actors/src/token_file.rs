//! Where an actor keeps its token (core/actors/token-file.js, and
//! docs/design/actors/interface.md, section 3): a file only you can read
//! (0600), named in XENONET_ACTOR_TOKEN_FILE, or the token itself in
//! XENONET_ACTOR_TOKEN. Never an argument: any program you run can read the
//! arguments of every other one with ps, and agents run as you. A file
//! others can read is refused, as ssh refuses loose keys, and one isn't
//! written inside a git working tree or the projects folder, where agents
//! read freely and tokens get committed, unless you say so.
//!
//! The doctor's look for tokens left in repos (findTokenFiles) moves with
//! the doctor.

use crate::tokens::is_token;
use std::path::Path;
use xenonet_base::fs::FsError;
use xenonet_base::paths::{is_under, path_key, resolve};
use xenonet_base::private_file::{acl_of, chmod_600, mode_of, others_in, protect_file, run_command, user_sid, Error, Run};
use xenonet_base::text::js_trim;

/// Folders of dependencies, builds and caches, whose ignored files are many
/// and none of them yours: the doctor's look past .gitignore leaves them
/// out.
pub const HEAVY_FOLDERS: [&str; 15] =
    ["node_modules", "bower_components", ".venv", "venv", ".tox", "__pycache__", ".gradle", "Pods", ".terraform", ".next", ".nuxt", ".turbo", ".cache", "target", "DerivedData"];

/// How long whoami or icacls may take when a token file is read or written.
pub const TOKEN_COMMAND_MS: u64 = 10000;

/// Runs whoami or icacls for the token files: the `run` to give them.
pub fn run_token_command(file: &str, args: &[&str]) -> Result<String, String> {
    run_command(file, args, std::time::Duration::from_millis(TOKEN_COMMAND_MS))
}

fn said(m: String) -> Error {
    Error::Said(m)
}

/// The token in a file, when only you can read it.
pub fn read_token_file(file: &str, windows: bool, run: &mut Run<'_>) -> Result<String, Error> {
    let st = std::fs::metadata(file).map_err(|_| said(format!("there's no token file at {file}")))?;
    if !st.is_file() {
        return Err(said(format!("{file} isn't a file")));
    }
    if windows {
        let others = (|| {
            let acl = acl_of(file, run)?;
            Ok::<_, Error>(others_in(&acl, &user_sid(run)?))
        })()
        .map_err(|_| {
            said(format!("XenoNet couldn't tell who else can read {file}, so it won't use the token in it; keep it in a folder of your own, such as one in your user folder"))
        })?;
        if !others.is_empty() {
            return Err(said(format!(
                "other users can read {file}, so XenoNet won't use the token in it; make it yours alone with icacls \"{file}\" /inheritance:r /grant:r \"%USERNAME%:F\""
            )));
        }
    } else if mode_of(&st) & 0o077 != 0 {
        return Err(said(format!("other users can read {file}, so XenoNet won't use the token in it; make it yours alone with chmod 600")));
    }
    let raw = std::fs::read(file).map_err(|e| Error::Fs(FsError::of(e, "open", file)))?;
    let token = js_trim(&String::from_utf8_lossy(&raw)).to_string();
    if !is_token(&token) {
        return Err(said(format!("{file} doesn't hold an actor's token")));
    }
    Ok(token)
}

/// The token an actor runs with, from its environment (`env` reads one
/// variable): the token and the file it came from, or None when it has
/// none.
pub fn token_from_env(env: &dyn Fn(&str) -> Option<String>, windows: bool, run: &mut Run<'_>) -> Result<Option<(String, Option<String>)>, Error> {
    if let Some(file) = env("XENONET_ACTOR_TOKEN_FILE").filter(|f| !f.is_empty()) {
        return Ok(Some((read_token_file(&file, windows, run)?, Some(file))));
    }
    if let Some(token) = env("XENONET_ACTOR_TOKEN").filter(|t| !t.is_empty()) {
        let token = js_trim(&token).to_string();
        if !is_token(&token) {
            return Err(said("XENONET_ACTOR_TOKEN doesn't hold an actor's token".into()));
        }
        return Ok(Some((token, None)));
    }
    Ok(None)
}

/// The git working tree holding a folder, or None.
pub fn git_tree_of(dir: &str) -> Option<String> {
    let full = resolve(dir);
    let mut d = Path::new(&full);
    loop {
        if d.join(".git").exists() {
            return Some(d.to_string_lossy().into_owned());
        }
        d = d.parent()?;
    }
}

/// Why a token shouldn't be written to `file`, or None when it may: a file
/// that's there already, or one inside a git working tree or the projects
/// folder. `force` says to go ahead anyway.
pub fn token_file_refusal(file: &str, force: bool, projects_dir: Option<&str>) -> Option<String> {
    if force {
        return None;
    }
    let full = resolve(file);
    if let Some(tree) = git_tree_of(&xenonet_base::nodepath::dirname(&full)) {
        return Some(format!("{full} is inside a git working tree ({tree}), where agents read freely and tokens get committed. Pick a file elsewhere, such as ~/.config/<name>/xenonet.token, or pass --force"));
    }
    let projects = projects_dir.filter(|p| !p.is_empty()).and_then(path_key);
    if path_key(&full).zip(projects).is_some_and(|(f, p)| is_under(&f, &p)) {
        return Some(format!("{full} is inside the projects folder, where agents read freely. Pick a file elsewhere, such as ~/.config/<name>/xenonet.token, or pass --force"));
    }
    if Path::new(&full).exists() {
        return Some(format!("{full} is there already; pass --force to replace it"));
    }
    None
}

fn fs_err(e: std::io::Error, syscall: &str, path: &str) -> Error {
    Error::Fs(FsError::of(e, syscall, path))
}

fn write_new(full: &str, text: &str) -> Result<(), Error> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    use std::io::Write;
    o.open(full).and_then(|mut f| f.write_all(text.as_bytes())).map_err(|e| fs_err(e, "open", full))
}

/// Writes a token to a file only you can read (0600, or on Windows an ACL
/// with your account alone and nothing inherited), in a folder made only
/// for you if it isn't there. Says where.
pub fn write_token_file(file: &str, token: &str, windows: bool, run: &mut Run<'_>, force: bool, projects_dir: Option<&str>) -> Result<String, Error> {
    if !is_token(token) {
        return Err(said("that isn't an actor's token".into()));
    }
    if let Some(why) = token_file_refusal(file, force, projects_dir) {
        return Err(said(why));
    }
    let full = resolve(file);
    let dir = xenonet_base::nodepath::dirname(&full);
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&dir).map_err(|e| fs_err(e, "mkdir", &dir))?;
    // On Windows the file is made new and empty, made yours alone (no
    // inherited entries, and your account's alone, which writing keeps),
    // and only then given the token. Elsewhere, a file that's there is
    // made yours alone before the token goes in.
    let remove = |full: &str| match std::fs::remove_file(full) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(fs_err(e, "rm", full)),
        _ => Ok(()),
    };
    if windows {
        remove(&full)?;
        write_new(&full, "")?;
        if let Err(e) = protect_file(&full, true, run) {
            remove(&full)?;
            let first = e.message().split('\n').next().unwrap_or("").to_string();
            return Err(said(format!("XenoNet couldn't make {full} yours alone, so it didn't write the token there ({first})")));
        }
        write_new(&full, &format!("{token}\n"))?;
        return Ok(full);
    }
    if Path::new(&full).exists() {
        chmod_600(Path::new(&full)).map_err(|e| fs_err(e, "chmod", &full))?;
    }
    write_new(&full, &format!("{token}\n"))?;
    chmod_600(Path::new(&full)).map_err(|e| fs_err(e, "chmod", &full))?;
    Ok(full)
}

// The tests of unix file modes are unix's alone; the made-up Windows runs
// everywhere. (cfg(unix) on each such test, not cfg(all(test, unix)):
// clippy takes only a plain cfg(test) for test code.)
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Made up, and split so the doctor's search for tokens left in repos
    // doesn't take this file for one that holds a real token.
    const TOKEN: &str = concat!("xenonet_", "actor_", "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    const SID: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";

    #[cfg(unix)]
    fn temp() -> String {
        let d = std::env::temp_dir().join(format!(
            "xenonet-token-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(d).unwrap().to_string_lossy().into_owned()
    }

    /// A made-up Windows whose icacls says a file's ACL is `acl` and
    /// whose whoami says this account is SID; `None` for a command that
    /// fails.
    fn windows(acl: Option<String>, whoami: bool) -> impl FnMut(&str, &[&str]) -> Result<String, String> {
        move |cmd: &str, args: &[&str]| match (cmd, args.get(1).copied()) {
            ("whoami", _) if whoami => Ok(format!("\"pc\\\\someone\",\"{SID}\"\r\n")),
            ("icacls", Some("/save")) if acl.is_some() => {
                let text = format!("{}\r\n{}\r\n", args[0], acl.as_deref().unwrap_or(""));
                let body: Vec<u8> = [0xff, 0xfe].into_iter().chain(text.encode_utf16().flat_map(u16::to_le_bytes)).collect();
                std::fs::write(args[2], body).unwrap();
                Ok(String::new())
            }
            _ => Err(format!("{cmd} failed\nwith more on the next line")),
        }
    }

    #[test]
    fn on_windows_a_token_file_is_used_only_when_its_acl_lets_no_one_else_read_it() {
        let dir = std::env::temp_dir().join(format!("xenonet-token-file-acl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("xenonet.token").to_string_lossy().into_owned();
        std::fs::write(&file, format!("{TOKEN}\n")).unwrap();
        let read = |acl: Option<String>, whoami: bool| read_token_file(&file, true, &mut windows(acl, whoami)).map_err(|e| e.message().to_string());
        for yours in [format!("D:P(A;;FA;;;{SID})"), format!("D:PAI(A;;FA;;;{SID})(A;;FA;;;SY)(A;;FA;;;BA)(A;OICIIO;GA;;;BU)(D;;FA;;;WD)")] {
            assert_eq!(read(Some(yours.clone()), true), Ok(TOKEN.to_string()), "{yours}");
        }
        let loose =
            format!("other users can read {file}, so XenoNet won't use the token in it; make it yours alone with icacls \"{file}\" /inheritance:r /grant:r \"%USERNAME%:F\"");
        for others in [format!("D:AI(A;ID;FA;;;{SID})(A;ID;0x1200a9;;;BU)"), "D:NO_ACCESS_CONTROL".into(), format!("D:(A;;FR;;;{SID}0)"), "D:(XA;;FR;;;AU)".into()] {
            assert_eq!(read(Some(others.clone()), true), Err(loose.clone()), "{others}");
        }
        // When it can't tell, it doesn't use the token.
        let blind = format!("XenoNet couldn't tell who else can read {file}, so it won't use the token in it; keep it in a folder of your own, such as one in your user folder");
        for (acl, whoami) in [(None, true), (Some(format!("D:P(A;;FA;;;{SID})")), false), (Some("no ACL here".to_string()), true)] {
            assert_eq!(read(acl, whoami), Err(blind.clone()));
        }
        // Nor one that isn't a file, or holds no token.
        assert_eq!(read_token_file(&dir.to_string_lossy(), true, &mut windows(None, true)).unwrap_err().message(), format!("{} isn't a file", dir.to_string_lossy()));
        std::fs::write(&file, "xenonet_actor_short\n").unwrap();
        assert_eq!(read(Some(format!("D:P(A;;FA;;;{SID})")), true), Err(format!("{file} doesn't hold an actor's token")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn on_windows_a_token_is_written_only_once_the_file_is_yours_alone() {
        let dir = std::env::temp_dir().join(format!("xenonet-token-file-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("wren").join("xenonet.token").to_string_lossy().into_owned();
        let full = resolve(&file);
        // What icacls was asked, and what the file held when it was.
        let asked = RefCell::new(Vec::new());
        let mut run = |cmd: &str, args: &[&str]| -> Result<String, String> {
            match cmd {
                "whoami" => Ok(format!("\"pc\\\\someone\",\"{SID}\"\r\n")),
                "icacls" => {
                    asked.borrow_mut().push((args.join(" "), std::fs::read_to_string(args[0]).unwrap()));
                    Ok(String::new())
                }
                _ => Err(format!("no {cmd} here")),
            }
        };
        assert_eq!(write_token_file(&file, "not a token", true, &mut run, false, None).unwrap_err().message(), "that isn't an actor's token");
        assert_eq!(write_token_file(&file, TOKEN, true, &mut run, false, None).unwrap(), full);
        assert_eq!(asked.borrow().clone(), [(format!("{full} /inheritance:r /grant:r *{SID}:F /q"), String::new())]);
        assert_eq!(std::fs::read_to_string(&full).unwrap(), format!("{TOKEN}\n"));
        assert_eq!(write_token_file(&file, TOKEN, true, &mut run, false, None).unwrap_err().message(), format!("{full} is there already; pass --force to replace it"));
        assert_eq!(write_token_file(&file, TOKEN, true, &mut run, true, None).unwrap(), full);
        // When it can't be made yours alone, no token is left in it.
        for whoami in [true, false] {
            let other = dir.join(format!("other-{whoami}.token")).to_string_lossy().into_owned();
            let e = write_token_file(&other, TOKEN, true, &mut windows(None, whoami), false, None).unwrap_err();
            let first = if whoami { "icacls failed" } else { "whoami failed" };
            assert_eq!(e.message(), format!("XenoNet couldn't make {} yours alone, so it didn't write the token there ({first})", resolve(&other)));
            assert!(!Path::new(&other).exists());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_token_from_the_environment() {
        let mut run = windows(None, false);
        let none = |_: &str| None;
        assert_eq!(token_from_env(&none, false, &mut run).unwrap(), None);
        let empty = |_: &str| Some(String::new());
        assert_eq!(token_from_env(&empty, false, &mut run).unwrap(), None);
        let bad = |k: &str| (k == "XENONET_ACTOR_TOKEN").then(|| "xenonet_actor_nope".to_string());
        assert_eq!(token_from_env(&bad, false, &mut run).unwrap_err().message(), "XENONET_ACTOR_TOKEN doesn't hold an actor's token");
        // The file, when both are set, and its own checks with it.
        let both = |k: &str| Some(if k == "XENONET_ACTOR_TOKEN_FILE" { "/no/such/xenonet.token".to_string() } else { TOKEN.to_string() });
        assert_eq!(token_from_env(&both, false, &mut run).unwrap_err().message(), "there's no token file at /no/such/xenonet.token");
    }

    #[cfg(unix)]
    #[test]
    fn written_yours_alone_and_read_back() {
        let dir = temp();
        let mut run = |_: &str, _: &[&str]| Err("no commands here".to_string());
        let file = format!("{dir}/config/wren/xenonet.token");
        assert_eq!(write_token_file(&file, TOKEN, false, &mut run, false, None).unwrap(), file);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(format!("{dir}/config")).unwrap().permissions().mode() & 0o077, 0);
        assert_eq!(read_token_file(&file, false, &mut run).unwrap(), TOKEN);
        assert_eq!(token_file_refusal(&file, false, None), Some(format!("{file} is there already; pass --force to replace it")));
        assert_eq!(
            token_file_refusal(&file, false, Some(&dir)).unwrap(),
            format!("{file} is inside the projects folder, where agents read freely. Pick a file elsewhere, such as ~/.config/<name>/xenonet.token, or pass --force")
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token_file(&file, false, &mut run).unwrap_err().message().ends_with("make it yours alone with chmod 600"));
        std::fs::create_dir(format!("{dir}/.git")).unwrap();
        assert_eq!(git_tree_of(&format!("{dir}/config/wren")), Some(dir.clone()));
        assert!(token_file_refusal(&format!("{dir}/a.token"), false, None).unwrap().contains("inside a git working tree"));
        assert_eq!(read_token_file(&format!("{dir}/none"), false, &mut run), Err(Error::Said(format!("there's no token file at {dir}/none"))));
        let env = |k: &str| (k == "XENONET_ACTOR_TOKEN").then(|| format!(" {TOKEN}\n"));
        assert_eq!(token_from_env(&env, false, &mut run).unwrap(), Some((TOKEN.to_string(), None)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_held_to_the_file_it_names_and_force_makes_a_loose_file_yours() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp();
        let mut run = |_: &str, _: &[&str]| Err("no commands here".to_string());
        let (loose, link) = (format!("{dir}/loose.token"), format!("{dir}/link.token"));
        std::fs::write(&loose, format!("{TOKEN}\n")).unwrap();
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&loose, &link).unwrap();
        // A link to a file others can read is refused as that file is.
        let refused = format!("other users can read {link}, so XenoNet won't use the token in it; make it yours alone with chmod 600");
        assert_eq!(read_token_file(&link, false, &mut run).unwrap_err().message(), refused);
        // Written again with --force, it's yours alone before the token goes in.
        assert_eq!(write_token_file(&loose, TOKEN, false, &mut run, true, None).unwrap(), loose);
        assert_eq!(std::fs::metadata(&loose).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(read_token_file(&link, false, &mut run).unwrap(), TOKEN);
        std::fs::write(&loose, "a note, not a token\n").unwrap();
        assert_eq!(read_token_file(&loose, false, &mut run).unwrap_err().message(), format!("{loose} doesn't hold an actor's token"));
        assert_eq!(read_token_file(&dir, false, &mut run).unwrap_err().message(), format!("{dir} isn't a file"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
