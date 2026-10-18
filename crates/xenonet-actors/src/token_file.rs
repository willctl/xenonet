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

fn parent_of(full: &str) -> String {
    Path::new(full).parent().map_or_else(|| full.to_string(), |p| p.to_string_lossy().into_owned())
}

/// Why a token shouldn't be written to `file`, or None when it may: a file
/// that's there already, or one inside a git working tree or the projects
/// folder. `force` says to go ahead anyway.
pub fn token_file_refusal(file: &str, force: bool, projects_dir: Option<&str>) -> Option<String> {
    if force {
        return None;
    }
    let full = resolve(file);
    if let Some(tree) = git_tree_of(&parent_of(&full)) {
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
    let dir = parent_of(&full);
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

#[cfg(test)]
mod tests {
    use super::*;

    // Made up, and split so the doctor's search for tokens left in repos
    // doesn't take this file for one that holds a real token.
    const TOKEN: &str = concat!("xenonet_", "actor_", "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");

    fn temp() -> String {
        let d = std::env::temp_dir().join(format!(
            "xenonet-token-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::canonicalize(d).unwrap().to_string_lossy().into_owned()
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
}
