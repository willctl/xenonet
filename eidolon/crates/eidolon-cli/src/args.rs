//! Reading the command line. Flags may come anywhere before a lone `--`;
//! what follows it is the receiving side's command, for `move`.

use crate::error::{CliError, Result};
use eidolon_browser::units::PROFILE_PATH;
use eidolon_model::{EidolonId, FacetKind};
use std::net::SocketAddr;
use std::path::PathBuf;

pub const USAGE: &str = "usage: eidolon [--json] <command>\n\
  machine init --name <name> [--key-store file]\n\
  machine show\n\
  new --wall <wall_x> --holder <holder.json|-> ... --folder <name>=<guest path> [--leave-out <glob>] ...\n\
  seal <eid> --cell <cell>\n\
  open <eid> --cell <cell>\n\
  move <eid> -- <command of the receiving side>\n\
  receive\n\
  move-settle <eid> --abandon | --confirm <seat.json|->\n\
  verify <eid>\n\
  facets <eid>\n\
  ls";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetArg {
    pub kind: FacetKind,
    pub name: String,
    pub path: String,
    pub leave_out: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    Abandon,
    Confirm(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Help,
    MachineInit { name: String, key_store: String },
    MachineShow,
    New { wall: String, holders: Vec<String>, facets: Vec<FacetArg> },
    Seal { eidolon: EidolonId, cell: Option<String> },
    Open { eidolon: EidolonId, cell: String },
    Move { eidolon: EidolonId, argv: Vec<String> },
    Receive,
    View { eidolon: EidolonId },
    Mcp { eidolon: EidolonId, http: Option<SocketAddr> },
    Settle { eidolon: EidolonId, how: Settle },
    Verify { eidolon: EidolonId },
    Facets { eidolon: EidolonId },
    Ls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub json: bool,
    /// For tests only: cells are folders under this one (see `cells`).
    pub cell_root: Option<PathBuf>,
    pub cmd: Cmd,
}

fn usage(message: &str) -> CliError {
    CliError::usage(message, "Run `eidolon help` for the commands.")
}

/// What is left of the arguments, taken from as each flag is read.
struct Rest(Vec<String>);

impl Rest {
    fn switch(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|a| a != name);
        self.0.len() != before
    }

    fn values(&mut self, name: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        while let Some(at) = self.0.iter().position(|a| a == name) {
            if at + 1 >= self.0.len() {
                return Err(usage(&format!("{name} needs a value.")));
            }
            self.0.remove(at);
            out.push(self.0.remove(at));
        }
        Ok(out)
    }

    fn value(&mut self, name: &str) -> Result<Option<String>> {
        let mut all = self.values(name)?;
        if all.len() > 1 {
            return Err(usage(&format!("{name} was given more than once.")));
        }
        Ok(all.pop())
    }

    fn need(&mut self, name: &str) -> Result<String> {
        self.value(name)?.ok_or_else(|| usage(&format!("{name} is missing.")))
    }

    /// The words that are not flags; any flag left over is a mistake.
    fn words(self) -> Result<Vec<String>> {
        match self.0.iter().find(|a| a.starts_with("--")) {
            Some(flag) => Err(usage(&format!("{flag} is not a flag of this command."))),
            None => Ok(self.0),
        }
    }
}

fn eid(words: &[String], at: usize) -> Result<EidolonId> {
    let text = words.get(at).ok_or_else(|| usage("The eidolon id is missing."))?;
    text.parse().map_err(|_| usage(&format!("{text:?} is not an eidolon id; it is `eid_` and 26 characters.")))
}

/// `--http` takes a loopback address and a port, so a mistake is caught
/// before any cell is touched.
fn http_addr(text: &str) -> Result<SocketAddr> {
    let addr: SocketAddr = text.parse().map_err(|_| usage(&format!("{text:?} is not an address; write it as 127.0.0.1:<port>.")))?;
    if !addr.ip().is_loopback() {
        return Err(usage(&format!("{text:?} is not on this machine. The tools are served on 127.0.0.1 only; XenoNet publishes them to other machines.")));
    }
    Ok(addr)
}

fn exactly(words: &[String], n: usize) -> Result<()> {
    if words.len() > n {
        Err(usage(&format!("{:?} is one word too many.", words[n])))
    } else {
        Ok(())
    }
}

pub fn parse(args: &[String]) -> Result<Parsed> {
    let (head, tail) = match args.iter().position(|a| a == "--") {
        Some(at) => (&args[..at], Some(args[at + 1..].to_vec())),
        None => (args, None),
    };
    let mut rest = Rest(head.to_vec());
    let json = rest.switch("--json");
    let cell_root = rest.value("--cell-root")?.map(PathBuf::from);
    if tail.is_some() && rest.0.first().map(String::as_str) != Some("move") {
        return Err(usage("`--` is only for `move`."));
    }
    let first = rest.0.first().cloned();
    let cmd = match first.as_deref() {
        None => return Err(usage("A command is missing.")),
        Some("help" | "--help" | "-h") => Cmd::Help,
        Some("machine") => {
            rest.0.remove(0);
            let sub = if rest.0.is_empty() { None } else { Some(rest.0.remove(0)) };
            match sub.as_deref() {
                Some("init") => {
                    let name = rest.need("--name")?;
                    let key_store = rest.value("--key-store")?.unwrap_or_else(|| "auto".to_owned());
                    exactly(&rest.words()?, 0)?;
                    Cmd::MachineInit { name, key_store }
                }
                Some("show") => {
                    exactly(&rest.words()?, 0)?;
                    Cmd::MachineShow
                }
                _ => return Err(usage("`machine` is followed by `init` or `show`.")),
            }
        }
        Some("new") => {
            rest.0.remove(0);
            new_args(rest)?
        }
        Some("seal") => {
            rest.0.remove(0);
            let cell = rest.value("--cell")?;
            let words = rest.words()?;
            exactly(&words, 1)?;
            Cmd::Seal { eidolon: eid(&words, 0)?, cell }
        }
        Some("open") => {
            rest.0.remove(0);
            let cell = rest.need("--cell")?;
            let words = rest.words()?;
            exactly(&words, 1)?;
            Cmd::Open { eidolon: eid(&words, 0)?, cell }
        }
        Some("move") => {
            rest.0.remove(0);
            let words = rest.words()?;
            exactly(&words, 1)?;
            let argv = tail.filter(|t| !t.is_empty()).ok_or_else(|| usage("`move` needs the receiving side's command after `--`."))?;
            Cmd::Move { eidolon: eid(&words, 0)?, argv }
        }
        Some("receive") => {
            rest.0.remove(0);
            exactly(&rest.words()?, 0)?;
            Cmd::Receive
        }
        Some("view") => {
            rest.0.remove(0);
            let words = rest.words()?;
            exactly(&words, 1)?;
            Cmd::View { eidolon: eid(&words, 0)? }
        }
        Some("mcp") => {
            rest.0.remove(0);
            let http = rest.value("--http")?.map(|a| http_addr(&a)).transpose()?;
            let words = rest.words()?;
            exactly(&words, 1)?;
            Cmd::Mcp { eidolon: eid(&words, 0)?, http }
        }
        Some("move-settle") => {
            rest.0.remove(0);
            let abandon = rest.switch("--abandon");
            let confirm = rest.value("--confirm")?;
            let words = rest.words()?;
            exactly(&words, 1)?;
            let how = match (abandon, confirm) {
                (true, None) => Settle::Abandon,
                (false, Some(seat)) => Settle::Confirm(seat),
                _ => return Err(usage("Give exactly one of --abandon and --confirm <seat.json>.")),
            };
            Cmd::Settle { eidolon: eid(&words, 0)?, how }
        }
        Some(name @ ("verify" | "facets")) => {
            let name = name.to_owned();
            rest.0.remove(0);
            let words = rest.words()?;
            exactly(&words, 1)?;
            let eidolon = eid(&words, 0)?;
            if name == "verify" {
                Cmd::Verify { eidolon }
            } else {
                Cmd::Facets { eidolon }
            }
        }
        Some("ls") => {
            rest.0.remove(0);
            exactly(&rest.words()?, 0)?;
            Cmd::Ls
        }
        Some(other) => return Err(usage(&format!("{other:?} is not a command."))),
    };
    Ok(Parsed { json, cell_root, cmd })
}

/// `new` reads its flags in order: a `--leave-out` belongs to the
/// `--folder` or `--browser` before it.
fn new_args(mut rest: Rest) -> Result<Cmd> {
    let wall = rest.need("--wall")?;
    let holders = rest.values("--holder")?;
    let mut facets: Vec<FacetArg> = Vec::new();
    let mut words = std::mem::take(&mut rest.0).into_iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--folder" => {
                let spec = words.next().ok_or_else(|| usage("--folder needs a value."))?;
                let (name, path) = spec.split_once('=').ok_or_else(|| usage("--folder is written <name>=<guest path>."))?;
                facets.push(FacetArg { kind: FacetKind::Folder, name: name.to_owned(), path: path.to_owned(), leave_out: Vec::new() });
            }
            "--browser" => {
                let name = words.next().ok_or_else(|| usage("--browser needs a name."))?;
                facets.push(FacetArg { kind: FacetKind::Browser, name, path: PROFILE_PATH.to_owned(), leave_out: Vec::new() });
            }
            "--leave-out" => {
                let pattern = words.next().ok_or_else(|| usage("--leave-out needs a value."))?;
                facets.last_mut().ok_or_else(|| usage("--leave-out comes after the --folder or --browser it belongs to."))?.leave_out.push(pattern);
            }
            other => return Err(usage(&format!("{other:?} is not part of this command."))),
        }
    }
    if holders.is_empty() {
        return Err(usage("--holder is missing; list at least this machine."));
    }
    Ok(Cmd::New { wall, holders, facets })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Result<Parsed> {
        parse(&line.split(' ').filter(|w| !w.is_empty()).map(str::to_owned).collect::<Vec<_>>())
    }

    const EID: &str = "eid_aeaqcaibaeaqcaibaeaqcaibae";

    #[test]
    fn flags_may_come_anywhere_and_json_is_global() {
        let a = p(&format!("seal --json {EID} --cell adytum-a")).unwrap();
        let b = p(&format!("--json seal --cell adytum-a {EID}")).unwrap();
        assert_eq!(a, b);
        assert!(a.json);
        assert!(matches!(a.cmd, Cmd::Seal { cell: Some(c), .. } if c == "adytum-a"));
    }

    #[test]
    fn a_leave_out_belongs_to_the_folder_before_it() {
        let line = "new --wall wall_x --holder - --folder a=/a --leave-out cache/** --leave-out *.tmp --folder b=/b";
        let Cmd::New { facets, .. } = p(line).unwrap().cmd else { panic!("not new") };
        assert_eq!(facets[0].leave_out, ["cache/**", "*.tmp"]);
        assert_eq!(facets[1].leave_out.len(), 0);
        assert_eq!(p("new --wall wall_x --holder - --leave-out x").unwrap_err().exit, 2);
    }

    #[test]
    fn a_browser_facet_lives_at_the_profile_path_and_takes_its_own_leave_outs() {
        let line = "new --wall wall_x --holder - --folder a=/a --browser web --leave-out extra/** --leave-out *.old";
        let Cmd::New { facets, .. } = p(line).unwrap().cmd else { panic!("not new") };
        assert_eq!((facets[1].kind, facets[1].name.as_str(), facets[1].path.as_str()), (FacetKind::Browser, "web", PROFILE_PATH));
        assert_eq!(facets[1].leave_out, ["extra/**", "*.old"]);
        assert_eq!(facets[0].leave_out.len(), 0);
        assert_eq!(p("new --wall wall_x --holder - --browser").unwrap_err().exit, 2);
    }

    #[test]
    fn view_and_mcp_read_their_arguments() {
        assert!(matches!(p(&format!("view {EID}")).unwrap().cmd, Cmd::View { .. }));
        assert!(matches!(p(&format!("mcp {EID}")).unwrap().cmd, Cmd::Mcp { http: None, .. }));
        assert!(matches!(p(&format!("mcp {EID} --http 127.0.0.1:9000")).unwrap().cmd, Cmd::Mcp { http: Some(h), .. } if h.port() == 9000));
        for line in [
            "view",
            &format!("view {EID} extra"),
            "mcp",
            &format!("mcp {EID} --http"),
            &format!("view {EID} --http x"),
            &format!("mcp {EID} --http 0.0.0.0:9000"),
            &format!("mcp {EID} --http example.com:80"),
            &format!("mcp {EID} --http 9000"),
        ] {
            assert_eq!(p(line).unwrap_err().exit, 2, "{line:?}");
        }
    }

    #[test]
    fn move_takes_everything_after_the_double_dash_untouched() {
        let args: Vec<String> = ["move", EID, "--", "ssh", "tower", "eidolon", "receive", "--json"].map(str::to_owned).to_vec();
        let Cmd::Move { argv, .. } = parse(&args).unwrap().cmd else { panic!("not move") };
        assert_eq!(argv, ["ssh", "tower", "eidolon", "receive", "--json"]);
        assert_eq!(p(&format!("move {EID}")).unwrap_err().exit, 2, "no receiving command");
        assert_eq!(p(&format!("seal {EID} --cell a -- x")).unwrap_err().exit, 2, "`--` is for move");
    }

    #[test]
    fn mistakes_are_usage_errors() {
        for line in
            ["", "nope", "machine", "machine init", "open eid_x --cell a", &format!("open {EID}"), &format!("verify {EID} --bad"), &format!("move-settle {EID}"), "ls extra"]
        {
            let e = p(line).unwrap_err();
            assert_eq!((e.exit, e.code.as_str()), (2, "usage"), "{line:?}");
        }
    }
}
