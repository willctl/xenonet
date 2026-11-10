//! Linux: the person's check for a device key that doesn't ask by itself.
//! The device key there is a file key, so the check is what stands between
//! another program of the account and the person's signature, and both
//! checks here are weaker than Touch ID (the person's chain records the
//! device as `weaker`):
//!
//! - polkit (`PolkitCheck`): `pkcheck` asks the session's authentication
//!   agent for the person's password. XenoNet's own action,
//!   `dev.xenonet.presence`, asks for the session owner's own password
//!   (`auth_self`) and nobody remote; it is there once a package installs
//!   `POLICY` as `ACTION_FILE`. Until then the check uses
//!   `org.freedesktop.policykit.exec`, pkexec's action, which every polkit
//!   has and which asks an administrator's password (`auth_admin`, kept for
//!   no time), the person's own on a one-person desktop. Weaker because an
//!   agent that can type at the session's dialog passes it, and because a
//!   rule an administrator writes can grant the action outright. That last
//!   case shows nobody anything, so the check first asks polkit without
//!   interaction and refuses to count an action it grants that way. polkit
//!   shows its action's own message, not the reason: the window shows the
//!   reason beside the dialog.
//! - a passphrase (`PassphraseCheck`), read from the controlling terminal
//!   (`/dev/tty`) with echo off, never from standard input, which an agent
//!   could write to. Weaker because its hash is in a file of this account,
//!   which a program of the account can read and try passphrases against,
//!   or replace. Used when polkit can't ask anyone (no agent, no session).
//!
//! Compiled on every unix so the tests run on any of them; only Linux and
//! the BSDs select it.

use ring::pbkdf2;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use xenonet_id::codec::{b64, random, unb64, unb64_n};
use xenonet_id::keys::write_private;
use xenonet_id::presence::UserCheck;
use xenonet_id::{IdError, R};

/// XenoNet's polkit action.
pub const ACTION: &str = "dev.xenonet.presence";
/// Where a package installs its definition.
pub const ACTION_FILE: &str = "/usr/share/polkit-1/actions/dev.xenonet.presence.policy";
/// The action every polkit has, used until XenoNet's is installed.
pub const EXEC_ACTION: &str = "org.freedesktop.policykit.exec";

/// XenoNet's action: the session owner's own password, each time, only
/// for an active local session.
pub const POLICY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
<policyconfig>
  <vendor>XenoNet</vendor>
  <vendor_url>https://github.com/willctl/xenonet</vendor_url>
  <action id="dev.xenonet.presence">
    <description>Confirm that you are at this computer</description>
    <message>XenoNet needs you to confirm an act you asked for</message>
    <defaults>
      <allow_any>no</allow_any>
      <allow_inactive>no</allow_inactive>
      <allow_active>auth_self</allow_active>
    </defaults>
  </action>
</policyconfig>
"#;

/// The process polkit asks about: `pid,start-time,uid`, the form pkcheck's
/// manual says new code uses, since the shorter forms look the rest up and
/// race a reused pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    pub pid: u32,
    /// Field 22 of `/proc/<pid>/stat`, in clock ticks since boot.
    pub start_time: u64,
    pub uid: u32,
}

impl Subject {
    /// The process `pid`, from `/proc`.
    pub fn of(pid: u32) -> R<Subject> {
        let read = |name: &str| std::fs::read_to_string(format!("/proc/{pid}/{name}")).map_err(|e| IdError::internal(format!("process {pid}'s {name}: {e}")));
        let start_time = parse_start_time(&read("stat")?).ok_or_else(|| IdError::internal(format!("process {pid}'s stat isn't readable")))?;
        let uid = parse_uid(&read("status")?).ok_or_else(|| IdError::internal(format!("process {pid}'s status isn't readable")))?;
        Ok(Subject { pid, start_time, uid })
    }

    pub fn arg(&self) -> String {
        format!("{},{},{}", self.pid, self.start_time, self.uid)
    }
}

/// The start time in a `/proc/<pid>/stat` line: field 22, counted after
/// the command's closing parenthesis, since the command may hold spaces
/// and parentheses itself.
pub fn parse_start_time(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // The fields after the command start at the third (the state).
    rest.split_whitespace().nth(22 - 3)?.parse().ok()
}

/// The real uid in a `/proc/<pid>/status`: the first number of `Uid:`.
pub fn parse_uid(status: &str) -> Option<u32> {
    status.lines().find_map(|l| l.strip_prefix("Uid:"))?.split_whitespace().next()?.parse().ok()
}

/// pkcheck's arguments: without interaction it only says whether the
/// action needs the person (exit 2) or is granted outright (exit 0).
pub fn pkcheck_args(action: &str, subject: &Subject, interactive: bool) -> Vec<String> {
    let mut args = vec!["--action-id".to_owned(), action.to_owned(), "--process".to_owned(), subject.arg()];
    if interactive {
        args.push("--allow-user-interaction".to_owned());
    }
    args
}

/// What pkcheck's exit code says (its manual's EXIT CODES).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// 0: authorized.
    Yes,
    /// 1: not authorized.
    No,
    /// 2: it needs the person, and no agent could ask or interaction
    /// wasn't allowed.
    NeedsPerson,
    /// 3: the person dismissed the dialog.
    Dismissed,
    /// 126, 127 or a signal: pkcheck failed.
    Failed,
}

pub fn answer(code: Option<i32>) -> Answer {
    match code {
        Some(0) => Answer::Yes,
        Some(1) => Answer::No,
        Some(2) => Answer::NeedsPerson,
        Some(3) => Answer::Dismissed,
        _ => Answer::Failed,
    }
}

/// Asks polkit whether the person is there, for the process `subject`:
/// the surface the person acts through, which sits in their session, where
/// polkit finds their authentication agent.
pub struct PolkitCheck {
    pkcheck: String,
    action: &'static str,
    subject: Subject,
    /// Asked when polkit can't ask anyone, never when the person dismissed
    /// its dialog or polkit refused.
    fallback: Option<PassphraseCheck>,
}

impl PolkitCheck {
    /// A check for the process `pid`, or None without pkcheck on the PATH.
    pub fn for_process(pid: u32, fallback: Option<PassphraseCheck>) -> R<Option<PolkitCheck>> {
        let Some(pkcheck) = xenonet_os::fx::on_path("pkcheck") else { return Ok(None) };
        let action = if Path::new(ACTION_FILE).exists() { ACTION } else { EXEC_ACTION };
        Ok(Some(PolkitCheck { pkcheck, action, subject: Subject::of(pid)?, fallback }))
    }

    pub fn action(&self) -> &'static str {
        self.action
    }

    fn ask(&self, interactive: bool) -> R<Answer> {
        // pkcheck runs only on Linux, where no window flashes; no_window keeps
        // the workspace's rule that every program XenoNet starts has it.
        let mut cmd = Command::new(&self.pkcheck);
        xenonet_os::process::Background::no_window(&mut cmd);
        let status = cmd
            .args(pkcheck_args(self.action, &self.subject, interactive))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| IdError::internal(format!("pkcheck: {e}")))?;
        Ok(answer(status.code()))
    }
}

impl UserCheck for PolkitCheck {
    fn confirm(&self, reason: &str) -> R<()> {
        match self.ask(false)? {
            Answer::NeedsPerson => {}
            Answer::Yes => return Err(IdError::refused(format!("polkit allows {} without asking anyone, so it can't show you're there", self.action))),
            Answer::No | Answer::Dismissed => return Err(IdError::refused(format!("polkit doesn't allow {}", self.action))),
            Answer::Failed => return Err(IdError::internal("pkcheck failed")),
        }
        match self.ask(true)? {
            Answer::Yes => Ok(()),
            Answer::NeedsPerson => match &self.fallback {
                Some(passphrase) => passphrase.confirm(reason),
                None => Err(IdError::refused("no polkit agent could ask you")),
            },
            Answer::Dismissed => Err(IdError::refused("you dismissed polkit's dialog")),
            Answer::No => Err(IdError::refused("polkit refused")),
            Answer::Failed => Err(IdError::internal("pkcheck failed")),
        }
    }
}

/// How hard the passphrase's hash is to try against: OWASP's 2023 figure
/// for PBKDF2-HMAC-SHA256.
const ITERATIONS: u32 = 600_000;
const SHORTEST: usize = 8;

/// A passphrase the person set at this device, kept as a salted PBKDF2
/// hash in an owner-only file, asked for on the controlling terminal.
pub struct PassphraseCheck {
    salt: [u8; 16],
    iterations: NonZeroU32,
    hash: [u8; 32],
}

impl PassphraseCheck {
    /// The passphrase in `file`, or None when none was set.
    pub fn load(file: &Path) -> R<Option<PassphraseCheck>> {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        PassphraseCheck::parse(&text).map(Some).ok_or_else(|| IdError::internal("the passphrase's file isn't readable"))
    }

    fn parse(text: &str) -> Option<PassphraseCheck> {
        let v: Value = serde_json::from_str(text).ok()?;
        (v["kdf"].as_str() == Some("pbkdf2-sha256")).then_some(())?;
        Some(PassphraseCheck {
            salt: unb64_n(v["salt"].as_str()?)?,
            iterations: NonZeroU32::new(u32::try_from(v["iterations"].as_u64()?).ok()?)?,
            hash: unb64(v["hash"].as_str()?)?.try_into().ok()?,
        })
    }

    /// Asks the person on the controlling terminal for a new passphrase,
    /// twice, and keeps it in `file`, which mustn't exist yet.
    pub fn set_from_terminal(file: &Path) -> R<PassphraseCheck> {
        let mut tty = Terminal::open()?;
        tty.say("Choose a passphrase XenoNet asks for when you approve an act at this device.")?;
        let first = tty.read_hidden("New passphrase: ")?;
        let again = tty.read_hidden("Again: ")?;
        if first != again {
            return Err(IdError::refused("the passphrases didn't match"));
        }
        PassphraseCheck::create(file, &first, ITERATIONS)
    }

    fn create(file: &Path, passphrase: &str, iterations: u32) -> R<PassphraseCheck> {
        if passphrase.chars().count() < SHORTEST {
            return Err(IdError::refused(format!("a passphrase has at least {SHORTEST} characters")));
        }
        let iterations = NonZeroU32::new(iterations).ok_or_else(|| IdError::internal("no iterations"))?;
        let salt = random::<16>()?;
        let mut hash = [0u8; 32];
        pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, iterations, &salt, passphrase.as_bytes(), &mut hash);
        let text = format!("{}\n", json!({ "v": 1, "kdf": "pbkdf2-sha256", "iterations": iterations.get(), "salt": b64(&salt), "hash": b64(&hash) }));
        write_private(file, text.as_bytes())?;
        Ok(PassphraseCheck { salt, iterations, hash })
    }

    /// Whether `passphrase` is the one set, compared in constant time.
    fn matches(&self, passphrase: &str) -> bool {
        pbkdf2::verify(pbkdf2::PBKDF2_HMAC_SHA256, self.iterations, &self.salt, passphrase.as_bytes(), &self.hash).is_ok()
    }
}

impl UserCheck for PassphraseCheck {
    fn confirm(&self, reason: &str) -> R<()> {
        let mut tty = Terminal::open()?;
        tty.say(&format!("XenoNet: {reason}"))?;
        let passphrase = tty.read_hidden("Passphrase: ")?;
        if self.matches(&passphrase) {
            Ok(())
        } else {
            Err(IdError::refused("that isn't the passphrase"))
        }
    }
}

/// The controlling terminal, opened by name: a process without one (a
/// service, an agent's tool run from pipes) gets an error, not stdin.
struct Terminal {
    file: std::fs::File,
    path: PathBuf,
}

impl Terminal {
    fn open() -> R<Terminal> {
        let path = PathBuf::from("/dev/tty");
        let file = std::fs::OpenOptions::new().read(true).write(true).open(&path).map_err(|_| IdError::refused("there's no terminal here to ask you at"))?;
        Ok(Terminal { file, path })
    }

    fn say(&mut self, line: &str) -> R<()> {
        writeln!(self.file, "{line}").map_err(|e| self.failed(&e))
    }

    /// A line typed with echo off.
    fn read_hidden(&mut self, prompt: &str) -> R<String> {
        write!(self.file, "{prompt}").and_then(|()| self.file.flush()).map_err(|e| self.failed(&e))?;
        let line = {
            let _quiet = Quiet::on(&self.file)?;
            let mut line = String::new();
            std::io::BufReader::new(&self.file).read_line(&mut line).map_err(|e| self.failed(&e))?;
            line
        };
        let _ = writeln!(self.file);
        Ok(line.trim_end_matches(['\n', '\r']).to_owned())
    }

    fn failed(&self, e: &std::io::Error) -> IdError {
        IdError::internal(format!("{}: {e}", self.path.display()))
    }
}

/// Echo off on a terminal until dropped.
struct Quiet {
    fd: std::os::fd::RawFd,
    before: libc::termios,
}

impl Quiet {
    #[expect(unsafe_code, reason = "tcgetattr(3) and tcsetattr(3), which std doesn't wrap")]
    fn on(file: &std::fs::File) -> R<Quiet> {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        // SAFETY: termios is plain data, so zeroed is a valid value for
        // tcgetattr to fill; `fd` is open for the whole call.
        let mut before: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: as above; `before` is a live termios it writes.
        if unsafe { libc::tcgetattr(fd, &raw mut before) } != 0 {
            return Err(IdError::internal("the terminal's settings can't be read"));
        }
        let mut quiet = before;
        quiet.c_lflag &= !libc::ECHO;
        quiet.c_lflag |= libc::ECHONL;
        // SAFETY: `quiet` is a live termios it reads; `fd` is open.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw const quiet) } != 0 {
            return Err(IdError::internal("the terminal's echo can't be turned off"));
        }
        Ok(Quiet { fd, before })
    }
}

impl Drop for Quiet {
    #[expect(unsafe_code, reason = "tcsetattr(3), which std doesn't wrap")]
    fn drop(&mut self) {
        // SAFETY: the terminal is still open (Quiet lives inside the
        // Terminal's borrow) and `before` is what tcgetattr filled.
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &raw const self.before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stat_line_gives_its_start_time_whatever_the_command_is() {
        // A command with spaces and a parenthesis of its own.
        let stat = "4242 (my (odd) cmd) S 1 4242 4242 0 -1 4194560 3000 0 0 0 12 3 0 0 20 0 4 0 987654 123456789 900 18446744073709551615";
        assert_eq!(parse_start_time(stat), Some(987_654));
        assert_eq!(parse_start_time("4242 (x) S 1"), None);
        assert_eq!(parse_start_time("no parenthesis"), None);
        assert_eq!(parse_uid("Name:\tx\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\n"), Some(1000));
        assert_eq!(parse_uid("Name:\tx\n"), None);
    }

    #[test]
    fn pkcheck_is_asked_about_one_process_by_pid_start_time_and_uid() {
        let subject = Subject { pid: 4242, start_time: 987_654, uid: 1000 };
        assert_eq!(pkcheck_args(ACTION, &subject, false), ["--action-id", "dev.xenonet.presence", "--process", "4242,987654,1000"]);
        assert_eq!(pkcheck_args(EXEC_ACTION, &subject, true), ["--action-id", "org.freedesktop.policykit.exec", "--process", "4242,987654,1000", "--allow-user-interaction"]);
        assert_eq!(answer(Some(0)), Answer::Yes);
        assert_eq!(answer(Some(1)), Answer::No);
        assert_eq!(answer(Some(2)), Answer::NeedsPerson);
        assert_eq!(answer(Some(3)), Answer::Dismissed);
        assert_eq!(answer(Some(127)), Answer::Failed);
        assert_eq!(answer(None), Answer::Failed);
        assert!(POLICY.contains(&format!("id=\"{ACTION}\"")) && POLICY.contains("<allow_active>auth_self</allow_active>"));
    }

    #[test]
    fn a_passphrase_is_kept_as_a_hash_and_matches_only_itself() {
        let dir = std::env::temp_dir().join(format!("xenonet-keys-pass-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("presence-passphrase.json");
        assert!(PassphraseCheck::load(&file).unwrap().is_none());
        assert!(PassphraseCheck::create(&file, "short", 1000).is_err(), "too short");
        let made = PassphraseCheck::create(&file, "correct horse", 1000).unwrap();
        assert!(made.matches("correct horse"));
        assert!(!made.matches("correct horsf"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("correct horse"));
        let loaded = PassphraseCheck::load(&file).unwrap().unwrap();
        assert!(loaded.matches("correct horse") && !loaded.matches(""));
        assert!(PassphraseCheck::create(&file, "another one", 1000).is_err(), "never replaced");
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, "{}").unwrap();
        assert!(PassphraseCheck::load(&file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
