//! How much a start is walled in: `off` (the app's own settings), `vendor`
//! (the app's own sandbox, asked for by the adapter) or `cell` (the app runs
//! inside an Adytum cell behind one wall, in a VM or container of its own).
//!
//! This file holds only the rules, as pure functions, and the trait the
//! engine calls to run a start in a cell. It names Adytum's wall type and
//! nothing of its drivers: the composition root (xenonet-service's
//! `cells`) implements [`CellHost`] on a driver, as it implements the
//! engine's other traits on the machine.
//!
//! The rules, each with its refusal test:
//! - A level is always named. A missing one is refused, never defaulted.
//! - `cell` names a wall, and no other level does.
//! - The wall is the starting identity's wall, derived from the identity's
//!   id by [`wall_for_identity`]. Another identity's wall is refused.
//! - The project folder is copied into the cell, never mounted, and the
//!   folder may not be the home folder or one that holds it.
//! - The cell gets an environment of its own: a fixed `PATH` and `HOME`
//!   inside the cell and the few names in [`ENV_ALLOWED`]. Any other name
//!   offered, and any value that carries the host's home folder, is
//!   refused. The service's token is in no allowed name.
//! - An app that has no build in the cell image is refused in words,
//!   whatever the request says. The program run is the image's, never the
//!   host's path to the app.

use adytum_model::Wall;
use sha2::{Digest, Sha256};
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The variables a cell may be given besides its fixed `PATH` and `HOME`:
/// how text and the terminal are shown, nothing that grants anything.
pub const ENV_ALLOWED: &[&str] = &["LANG", "LC_ALL", "LC_CTYPE", "TERM", "TZ", "NO_COLOR"];

/// `PATH` inside every cell: the image's own folders, none of the host's.
pub const CELL_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// `HOME` inside every cell: a folder of the cell's, made by the host and
/// never the user's home folder.
pub const CELL_HOME: &str = "/tmp/xn-home";

/// How much a start is walled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confinement {
    /// The app's own settings; no sandbox is asked for.
    Off,
    /// The app's own sandbox, which the adapter turns on.
    Vendor,
    /// Inside a cell behind this wall.
    Cell { wall: Wall },
}

/// Why a start was not confined, or could not run confined. Each reads as a
/// sentence that says what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfineError {
    /// No level was named.
    LevelMissing,
    /// A level other than off, vendor or cell.
    LevelUnknown(String),
    /// `cell` without a wall.
    WallMissing,
    /// A wall with off or vendor.
    WallNotAllowed(String),
    /// A wall that is not written as a wall.
    WallMalformed(String),
    /// A wall that is not the starting identity's.
    WallNotIdentitys { wall: String, expected: String },
    /// An identity id that is empty.
    IdentityMissing,
    /// A start at `off` or `vendor` was sent to a cell.
    NotACellStart,
    /// An app with no build in the cell image.
    NoLinuxBuild { app: String },
    /// A variable outside the allowlist.
    EnvNotAllowed(String),
    /// A value that holds the host's home folder, or a NUL.
    EnvValueRefused(String),
    /// A project folder that is the home folder or holds it, or is not absolute.
    ProjectRefused { project: String, why: &'static str },
    /// The host failed to run it in the cell.
    Host(String),
}

impl fmt::Display for ConfineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LevelMissing => f.write_str("The start does not say how confined it is. Name one: off, vendor or cell."),
            Self::LevelUnknown(l) => write!(f, "{l:?} is not a confinement level. Use off, vendor or cell."),
            Self::WallMissing => f.write_str("A start in a cell needs the wall of the identity it runs as. Name the wall, or choose off or vendor."),
            Self::WallNotAllowed(l) => write!(f, "A wall only goes with the cell level, and this start is {l}. Leave the wall out, or choose cell."),
            Self::WallMalformed(w) => write!(f, "{w:?} is not a wall. A wall is written wall_ and then lowercase letters, digits, _ or -."),
            Self::WallNotIdentitys { wall, expected } => {
                write!(f, "The wall {wall} is not this identity's. A start runs behind its own identity's wall, which is {expected}.")
            }
            Self::IdentityMissing => f.write_str("A start in a cell needs the identity it runs as, to find its wall. Name the identity."),
            Self::NotACellStart => f.write_str("Only a start at the cell level runs in a cell. Choose cell, or run it the way its level says."),
            Self::NoLinuxBuild { app } => {
                write!(f, "{app} has no Linux build in the cell image, so it cannot run in a cell. Start it with vendor confinement or off, or add it to the image.")
            }
            Self::EnvNotAllowed(n) => write!(f, "A cell is not given {n}. It gets only {}; leave {n} out.", ENV_ALLOWED.join(", ")),
            Self::EnvValueRefused(n) => write!(f, "The value of {n} holds the host's home folder or a NUL byte, which a cell does not get. Leave it out."),
            Self::ProjectRefused { project, why } => write!(f, "The project folder {project} cannot be copied into a cell: {why}."),
            Self::Host(m) => write!(f, "The cell could not run the start: {m}"),
        }
    }
}

impl std::error::Error for ConfineError {}

impl Confinement {
    /// The level a request names, with the wall it names for `cell`. A level
    /// is never defaulted: `None` is refused.
    pub fn parse(level: Option<&str>, wall: Option<&str>) -> Result<Self, ConfineError> {
        let level = level.filter(|l| !l.is_empty()).ok_or(ConfineError::LevelMissing)?;
        match (level, wall.filter(|w| !w.is_empty())) {
            ("cell", None) => Err(ConfineError::WallMissing),
            ("cell", Some(w)) => w.parse::<Wall>().map(|wall| Self::Cell { wall }).map_err(|_| ConfineError::WallMalformed(w.to_owned())),
            ("off" | "vendor", Some(_)) => Err(ConfineError::WallNotAllowed(level.to_owned())),
            ("off", None) => Ok(Self::Off),
            ("vendor", None) => Ok(Self::Vendor),
            (other, _) => Err(ConfineError::LevelUnknown(other.to_owned())),
        }
    }
}

/// The wall of an identity, from its id (`claude:<account>:<org>`,
/// `github:<host>/<login>`…). The rule: `wall_`, the id as a lowercase slug
/// (anything outside a-z and 0-9 becomes one `-`, at most 32 characters),
/// `_`, and the first 16 hex digits of the id's SHA-256. The hash is of the
/// id as written, so two ids that slug alike still get two walls.
pub fn wall_for_identity(identity_id: &str) -> Result<Wall, ConfineError> {
    if identity_id.trim().is_empty() {
        return Err(ConfineError::IdentityMissing);
    }
    let mut slug = String::new();
    for c in identity_id.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(32).collect();
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "id" } else { slug };
    let digest = Sha256::digest(identity_id.as_bytes());
    let hash = digest.iter().take(8).fold(String::new(), |mut h, b| {
        let _ = write!(h, "{b:02x}");
        h
    });
    let wall = format!("wall_{slug}_{hash}");
    wall.parse::<Wall>().map_err(|_| ConfineError::WallMalformed(wall))
}

impl Confinement {
    /// Whether this level fits the identity the start runs as: a cell's wall
    /// must be that identity's. Off and vendor name no wall.
    pub fn check_identity(&self, identity_id: &str) -> Result<(), ConfineError> {
        match self {
            Self::Cell { wall } => {
                let expected = wall_for_identity(identity_id)?;
                if *wall == expected {
                    Ok(())
                } else {
                    Err(ConfineError::WallNotIdentitys { wall: wall.to_string(), expected: expected.to_string() })
                }
            }
            Self::Off | Self::Vendor => Ok(()),
        }
    }
}

/// The variables offered to a cell, checked: each name must be in
/// [`ENV_ALLOWED`], and no value may hold the host's home folder or a NUL.
/// Nothing is dropped quietly; a refused name is named.
pub fn cell_env(offered: &[(String, String)], host_home: &str) -> Result<Vec<(String, String)>, ConfineError> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (name, value) in offered {
        if !ENV_ALLOWED.contains(&name.as_str()) {
            return Err(ConfineError::EnvNotAllowed(name.clone()));
        }
        if value.contains('\0') || holds_home(value, host_home) {
            return Err(ConfineError::EnvValueRefused(name.clone()));
        }
        if !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), value.clone()));
        }
    }
    Ok(out)
}

/// What a host's own environment gives a cell: only the allowed names, and
/// none whose value holds the home folder. For a caller that has the whole
/// environment and wants the part a cell may have.
pub fn allowed_from_host(host_env: impl IntoIterator<Item = (String, String)>, host_home: &str) -> Vec<(String, String)> {
    host_env.into_iter().filter(|(n, v)| ENV_ALLOWED.contains(&n.as_str()) && !v.contains('\0') && !holds_home(v, host_home)).collect()
}

fn holds_home(value: &str, home: &str) -> bool {
    let home = home.trim_end_matches(['/', '\\']);
    // A home of "" or "/" would match every value or path.
    home.len() > 1 && value.contains(home)
}

/// An app the cell image can run: the app's id and the program in the image
/// that is it (an absolute path inside the image).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellApp {
    pub app: String,
    pub program: String,
}

/// A start, as the engine hands it to a cell.
#[derive(Clone, Debug)]
pub struct CellRequest<'a> {
    pub confinement: &'a Confinement,
    /// The identity the start runs as (`identity/v1`'s id).
    pub identity_id: &'a str,
    pub app: &'a str,
    /// The arguments after the program. The host's own path to the app is
    /// not here: the program run is the image's.
    pub args: &'a [String],
    /// On standard input (the first prompt), never an argument.
    pub stdin: Option<&'a [u8]>,
    /// The project folder on the host, copied in.
    pub project: &'a Path,
    pub offered_env: &'a [(String, String)],
    pub host_home: &'a str,
    /// Where the project copy is copied back to when the run ends.
    pub copy_out: Option<&'a Path>,
    /// Keep the cell when the run ends (default: remove it).
    pub keep: bool,
    pub deadline: Option<Duration>,
}

/// A start that passed every rule, ready to run in a cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellPlan {
    pub wall: Wall,
    pub app: String,
    /// The program inside the image.
    pub program: String,
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    /// The environment the program gets, all of it: `PATH`, `HOME`, then the
    /// allowed names.
    pub env: Vec<(String, String)>,
    pub project: PathBuf,
    pub copy_out: Option<PathBuf>,
    pub keep: bool,
    pub deadline: Option<Duration>,
}

/// What a run in a cell left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellRun {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    /// The cell's name, for the record.
    pub cell: String,
    /// Whether the cell was removed when the run ended.
    pub removed: bool,
}

/// What the engine needs of a place that can run a start in a cell.
pub trait CellHost: Send + Sync {
    /// The apps the cell image holds, each with its program in the image.
    fn apps(&self) -> Vec<CellApp>;

    /// Runs a plan that passed [`plan`]: makes the cell behind the plan's
    /// wall, copies the project in, runs the program with exactly the plan's
    /// environment, copies the result out if asked, and removes the cell
    /// unless the plan keeps it.
    fn run(&self, plan: &CellPlan) -> Result<CellRun, ConfineError>;
}

/// Checks every rule for a start and makes its plan. Nothing is run or
/// written.
pub fn plan(host: &dyn CellHost, req: &CellRequest<'_>) -> Result<CellPlan, ConfineError> {
    let Confinement::Cell { wall } = req.confinement else {
        return Err(ConfineError::NotACellStart);
    };
    req.confinement.check_identity(req.identity_id)?;
    let program = host
        .apps()
        .into_iter()
        .find(|a| a.app == req.app && a.program.starts_with('/'))
        .map(|a| a.program)
        .ok_or_else(|| ConfineError::NoLinuxBuild { app: req.app.to_owned() })?;
    check_project(req.project, req.host_home)?;
    let mut env = vec![("PATH".to_owned(), CELL_PATH.to_owned()), ("HOME".to_owned(), CELL_HOME.to_owned())];
    env.extend(cell_env(req.offered_env, req.host_home)?);
    if req.args.iter().any(|a| a.contains('\0')) {
        return Err(ConfineError::EnvValueRefused("an argument".to_owned()));
    }
    Ok(CellPlan {
        wall: wall.clone(),
        app: req.app.to_owned(),
        program,
        args: req.args.to_vec(),
        stdin: req.stdin.map(<[u8]>::to_vec),
        env,
        project: req.project.to_path_buf(),
        copy_out: req.copy_out.map(Path::to_path_buf),
        keep: req.keep,
        deadline: req.deadline,
    })
}

/// Plans a start and runs it in the cell.
pub fn run_confined(host: &dyn CellHost, req: &CellRequest<'_>) -> Result<CellRun, ConfineError> {
    host.run(&plan(host, req)?)
}

fn check_project(project: &Path, host_home: &str) -> Result<(), ConfineError> {
    let refuse = |why| Err(ConfineError::ProjectRefused { project: project.display().to_string(), why });
    if !project.is_absolute() {
        return refuse("it is not an absolute path");
    }
    let home = Path::new(host_home.trim_end_matches(['/', '\\']));
    // The home folder itself, or any folder that holds it (including /),
    // would put the user's other projects, keys and settings in the cell.
    if !host_home.is_empty() && home.starts_with(project) {
        return refuse("it is the home folder or holds it, and a cell never gets the home folder");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const HOME: &str = "/Users/nova";

    struct Fake {
        apps: Vec<CellApp>,
        ran: Mutex<Vec<CellPlan>>,
    }

    impl Fake {
        fn with(apps: &[(&str, &str)]) -> Self {
            Self { apps: apps.iter().map(|(a, p)| CellApp { app: (*a).into(), program: (*p).into() }).collect(), ran: Mutex::new(Vec::new()) }
        }
    }

    impl CellHost for Fake {
        fn apps(&self) -> Vec<CellApp> {
            self.apps.clone()
        }

        fn run(&self, plan: &CellPlan) -> Result<CellRun, ConfineError> {
            self.ran.lock().expect("a test's lock").push(plan.clone());
            Ok(CellRun { status: Some(0), stdout: Vec::new(), stderr: Vec::new(), timed_out: false, cell: "adytum-test-1".into(), removed: true })
        }
    }

    fn cell_for(identity: &str) -> Confinement {
        Confinement::Cell { wall: wall_for_identity(identity).expect("a wall") }
    }

    fn request<'a>(c: &'a Confinement, identity: &'a str, project: &'a Path) -> CellRequest<'a> {
        CellRequest {
            confinement: c,
            identity_id: identity,
            app: "fake-agent",
            args: &[],
            stdin: None,
            project,
            offered_env: &[],
            host_home: HOME,
            copy_out: None,
            keep: false,
            deadline: None,
        }
    }

    #[test]
    fn a_missing_level_is_refused_never_defaulted() {
        assert_eq!(Confinement::parse(None, None), Err(ConfineError::LevelMissing));
        assert_eq!(Confinement::parse(Some(""), None), Err(ConfineError::LevelMissing));
        assert_eq!(Confinement::parse(None, Some("wall_a")), Err(ConfineError::LevelMissing));
    }

    #[test]
    fn a_cell_start_without_a_wall_is_refused() {
        assert_eq!(Confinement::parse(Some("cell"), None), Err(ConfineError::WallMissing));
        assert_eq!(Confinement::parse(Some("cell"), Some("")), Err(ConfineError::WallMissing));
        assert!(ConfineError::WallMissing.to_string().contains("needs the wall"));
    }

    #[test]
    fn a_wall_with_any_other_level_is_refused() {
        for level in ["off", "vendor"] {
            assert_eq!(Confinement::parse(Some(level), Some("wall_a")), Err(ConfineError::WallNotAllowed(level.into())));
        }
    }

    #[test]
    fn an_unknown_level_or_a_malformed_wall_is_refused() {
        assert_eq!(Confinement::parse(Some("strict"), None), Err(ConfineError::LevelUnknown("strict".into())));
        assert_eq!(Confinement::parse(Some("Cell"), Some("wall_a")), Err(ConfineError::LevelUnknown("Cell".into())));
        assert_eq!(Confinement::parse(Some("cell"), Some("work")), Err(ConfineError::WallMalformed("work".into())));
    }

    #[test]
    fn the_levels_parse() {
        assert_eq!(Confinement::parse(Some("off"), None), Ok(Confinement::Off));
        assert_eq!(Confinement::parse(Some("vendor"), None), Ok(Confinement::Vendor));
        assert!(matches!(Confinement::parse(Some("cell"), Some("wall_a")), Ok(Confinement::Cell { .. })));
    }

    #[test]
    fn another_identitys_wall_is_refused() {
        let theirs = cell_for("github:github.com/other");
        assert!(matches!(theirs.check_identity("github:github.com/nova"), Err(ConfineError::WallNotIdentitys { .. })));
        let fake = Fake::with(&[("fake-agent", "/bin/sh")]);
        let req = request(&theirs, "github:github.com/nova", Path::new("/work/a"));
        assert!(matches!(plan(&fake, &req), Err(ConfineError::WallNotIdentitys { .. })));
        assert!(fake.ran.lock().expect("lock").is_empty());
    }

    #[test]
    fn an_identitys_own_wall_passes() {
        let mine = cell_for("github:github.com/nova");
        assert_eq!(mine.check_identity("github:github.com/nova"), Ok(()));
        assert_eq!(Confinement::Off.check_identity("anything"), Ok(()));
    }

    #[test]
    fn the_wall_for_an_identity_is_derived_the_documented_way() {
        let w = wall_for_identity("claude:Nova@Example.com:Atlas Org").expect("a wall");
        let text = w.to_string();
        assert!(text.starts_with("wall_claude-nova-example-com-atlas-or_"), "{text}");
        assert_eq!(text.len(), "wall_claude-nova-example-com-atlas-or_".len() + 16);
        assert_eq!(wall_for_identity("claude:Nova@Example.com:Atlas Org").expect("again"), w, "the same id gives the same wall");
    }

    #[test]
    fn ids_that_slug_alike_get_different_walls() {
        let a = wall_for_identity("git:a.b").expect("a");
        let b = wall_for_identity("git:a-b").expect("b");
        let c = wall_for_identity("GIT:A_B").expect("c");
        assert!(a != b && b != c && a != c);
    }

    #[test]
    fn a_long_or_odd_id_still_gives_a_valid_wall() {
        for id in [&"x".repeat(500)[..], "::::", "é", "a b\tc"] {
            let w = wall_for_identity(id).expect(id);
            assert!(w.as_str().len() <= 5 + 64, "{w}");
        }
        assert_eq!(wall_for_identity(""), Err(ConfineError::IdentityMissing));
        assert_eq!(wall_for_identity("  "), Err(ConfineError::IdentityMissing));
    }

    #[test]
    fn a_start_at_off_or_vendor_is_not_sent_to_a_cell() {
        let fake = Fake::with(&[("fake-agent", "/bin/sh")]);
        for c in [Confinement::Off, Confinement::Vendor] {
            assert_eq!(plan(&fake, &request(&c, "git:x", Path::new("/work/a"))), Err(ConfineError::NotACellStart));
        }
    }

    #[test]
    fn an_app_with_no_linux_build_is_refused_in_words() {
        let c = cell_for("git:x");
        let fake = Fake::with(&[("other", "/usr/bin/other")]);
        let err = plan(&fake, &request(&c, "git:x", Path::new("/work/a"))).expect_err("refused");
        assert_eq!(err, ConfineError::NoLinuxBuild { app: "fake-agent".into() });
        assert!(err.to_string().contains("no Linux build in the cell image"));
        assert!(err.to_string().contains("vendor confinement or off"));
        // A program that is not an absolute path in the image is no build either.
        let fake = Fake::with(&[("fake-agent", "agent")]);
        assert!(matches!(plan(&fake, &request(&c, "git:x", Path::new("/work/a"))), Err(ConfineError::NoLinuxBuild { .. })));
    }

    #[test]
    fn a_variable_outside_the_allowlist_is_refused() {
        for name in ["HOME", "PATH", "XENONET_TOKEN", "ANTHROPIC_API_KEY", "GITHUB_TOKEN", "SSH_AUTH_SOCK", "USER"] {
            let offered = [(name.to_owned(), "x".to_owned())];
            assert_eq!(cell_env(&offered, HOME), Err(ConfineError::EnvNotAllowed(name.into())), "{name}");
        }
        let offered = [("LANG".to_owned(), "C.UTF-8".to_owned())];
        assert_eq!(cell_env(&offered, HOME), Ok(offered.to_vec()));
    }

    #[test]
    fn a_value_with_the_host_home_is_refused_even_in_an_allowed_name() {
        for value in ["/Users/nova", "/Users/nova/.config", "x:/Users/nova/bin", "a\0b"] {
            let offered = [("TZ".to_owned(), value.to_owned())];
            assert_eq!(cell_env(&offered, HOME), Err(ConfineError::EnvValueRefused("TZ".into())), "{value:?}");
        }
    }

    #[test]
    fn the_hosts_whole_environment_is_cut_to_the_allowlist() {
        let host = [("HOME", "/Users/nova"), ("PATH", "/opt/homebrew/bin"), ("XENONET_TOKEN", "secret"), ("LANG", "en_US.UTF-8"), ("TERM", "xterm"), ("TZ", "/Users/nova/zone")]
            .map(|(n, v)| (n.to_owned(), v.to_owned()));
        let kept = allowed_from_host(host, HOME);
        assert_eq!(kept, vec![("LANG".to_owned(), "en_US.UTF-8".to_owned()), ("TERM".to_owned(), "xterm".to_owned())]);
    }

    #[test]
    fn the_home_folder_and_its_parents_are_not_copied_in() {
        let c = cell_for("git:x");
        let fake = Fake::with(&[("fake-agent", "/bin/sh")]);
        for project in ["/Users/nova", "/Users/nova/", "/Users", "/"] {
            let err = plan(&fake, &request(&c, "git:x", Path::new(project))).expect_err(project);
            assert!(matches!(err, ConfineError::ProjectRefused { .. }), "{project}");
        }
        let err = plan(&fake, &request(&c, "git:x", Path::new("work/a"))).expect_err("relative");
        assert!(matches!(err, ConfineError::ProjectRefused { .. }));
        assert!(plan(&fake, &request(&c, "git:x", Path::new("/Users/nova/Projects/atlas"))).is_ok(), "a project inside the home folder is copied, not mounted");
        assert!(fake.ran.lock().expect("lock").is_empty());
    }

    #[test]
    fn a_plan_has_exactly_the_cells_environment() {
        let c = cell_for("git:x");
        let fake = Fake::with(&[("fake-agent", "/bin/sh")]);
        let offered = [("LANG".to_owned(), "C.UTF-8".to_owned())];
        let args = ["agent.sh".to_owned()];
        let mut req = request(&c, "git:x", Path::new("/work/a"));
        req.offered_env = &offered;
        req.args = &args;
        let run = run_confined(&fake, &req).expect("runs");
        assert!(run.removed);
        let ran = fake.ran.lock().expect("lock");
        let plan = ran.first().expect("one plan");
        assert_eq!(plan.program, "/bin/sh");
        assert_eq!(plan.args, ["agent.sh"]);
        let names: Vec<&str> = plan.env.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["PATH", "HOME", "LANG"]);
        assert_eq!(plan.env[1].1, CELL_HOME);
        assert!(!plan.env.iter().any(|(_, v)| v.contains(HOME)));
        assert_eq!(plan.wall, wall_for_identity("git:x").expect("wall"));
    }

    #[test]
    fn a_refused_start_runs_nothing() {
        let c = cell_for("git:x");
        let fake = Fake::with(&[]);
        assert!(run_confined(&fake, &request(&c, "git:x", Path::new("/work/a"))).is_err());
        assert!(fake.ran.lock().expect("lock").is_empty());
    }
}
