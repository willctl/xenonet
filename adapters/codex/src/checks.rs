//! Codex's doctor checks: the command (the ChatGPT app's or the CLI), its
//! sign-in, the sessions folder, and whether its threads are connected.

use async_trait::async_trait;
use xenonet_adapter::check::{connected, folder, privacy_fix, s, Facts, Folder};
use xenonet_adapter::{Check, CheckCtx, CheckRow, Slot, Stop, Verdict};
use xenonet_base::nodepath::{style_of, NATIVE};
use xenonet_base::text::{js_string, truthy};

/// The checks, in the order they show.
pub fn all() -> Vec<Box<dyn Check>> {
    vec![Box::new(Cli), Box::new(Auth), Box::new(Sessions), Box::new(Threads)]
}

const GROUP: &str = "Codex";

struct Cli;
struct Auth;
struct Sessions;
struct Threads;

/// Codex's command: the ChatGPT app's codex (`codexBin`, when it's there)
/// or a codex on PATH, or None when there's neither. That's Codex not
/// installed, which is no problem: XenoNet shows its threads once it is.
pub async fn codex_bin(c: &dyn CheckCtx) -> Result<Option<String>, Stop> {
    let mut bins: Vec<String> = Vec::new();
    if let Some(b) = Facts(c.facts()).get("codexBin").filter(|v| truthy(Some(v))) {
        bins.push(js_string(b));
    }
    if let Some(b) = c.which("codex").await? {
        bins.push(b);
    }
    let mut bin = None;
    for b in &bins {
        if NATIVE.is_absolute(b) && c.exists(b).await? {
            bin = Some(b.clone());
            break;
        }
    }
    Ok(match bin.or_else(|| bins.first().cloned()) {
        Some(b) if NATIVE.is_absolute(&b) => c.exists(&b).await?.then_some(b),
        Some(b) => c.which(&b).await?.is_some().then_some(b),
        None => None,
    })
}

#[async_trait(?Send)]
impl Check for Cli {
    fn row(&self) -> CheckRow {
        CheckRow { id: "codex.cli", group: GROUP, title: "Codex is installed", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let Some(bin) = codex_bin(c).await? else {
            return Ok(Verdict::info("not installed; XenoNet shows Codex's threads once it is", Some("To use Codex, install the ChatGPT desktop app or the Codex CLI.")));
        };
        let v = c.run(&bin, &["--version"], 4000.0).await?;
        let version = re!(r"\d+\.\d+\.\d+[A-Za-z0-9_.-]*").find(&s(v.get("stdout"))).map(|m| m.as_str().to_string());
        Ok(Verdict::ok(format!("{}{}", version.map_or(String::new(), |v| format!("version {v}, ")), c.tilde(&bin))))
    }
}

#[async_trait(?Send)]
impl Check for Auth {
    fn row(&self) -> CheckRow {
        CheckRow { id: "codex.auth", group: GROUP, title: "Codex is signed in", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let bin = match c.which("codex").await? {
            Some(b) => Some(b),
            None => codex_bin(c).await?,
        };
        let Some(bin) = bin else { return Ok(Verdict::skip("Codex isn't installed")) };
        let r = c.run(&bin, &["login", "status"], 6000.0).await?;
        let text = xenonet_base::text::js_trim(&format!("{}{}", s(r.get("stdout")), s(r.get("stderr")))).to_string();
        let lower = text.to_ascii_lowercase();
        let first = text.split('\n').next().unwrap_or("").to_string();
        if lower.contains("logged in") && !lower.contains("not logged in") {
            return Ok(Verdict::ok(match first.strip_prefix("Logged in") {
                Some(rest) => format!("signed in{rest}"),
                None => first,
            }));
        }
        if lower.contains("not logged in") {
            return Ok(Verdict::fail("not signed in", Some("Run codex login.")));
        }
        Ok(Verdict::warn(if text.is_empty() { "couldn't tell from codex login status".to_string() } else { first }, None))
    }
}

#[async_trait(?Send)]
impl Check for Sessions {
    fn row(&self) -> CheckRow {
        CheckRow { id: "codex.sessions", group: GROUP, title: "XenoNet can read Codex's sessions", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let home = Facts(c.facts()).get("paths").and_then(|p| p.get("codexHome")).filter(|v| !v.is_null()).map(js_string);
        let dir = home.as_ref().map(|h| if h.is_empty() { String::new() } else { style_of(h).join(&[h, "sessions"]) });
        let shown = c.tilde(&dir.clone().unwrap_or_else(|| "undefined".into()));
        Ok(match folder(c, dir.as_deref()).await? {
            Folder::Missing => Verdict::info(format!("no {shown} yet: no Codex sessions on this machine"), None),
            Folder::Denied(code) => Verdict::fail(format!("can't read {shown} ({code})"), Some(&privacy_fix(c, &shown))),
            Folder::Ok(_) => Verdict::ok(shown),
        })
    }
}

#[async_trait(?Send)]
impl Check for Threads {
    fn row(&self) -> CheckRow {
        CheckRow { id: "agents.codex", group: "Sessions", title: "Codex sessions are connected to XenoNet", slot: Slot::Sessions }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        Ok(connected(c, "codex", "Codex"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use xenonet_adapter::check::fake::{block_on, FakeCtx};
    use xenonet_adapter::check::Status;

    fn facts() -> Value {
        json!({ "home": "/Users/ada", "platform": "darwin", "paths": { "codexHome": "/Users/ada/.codex" } })
    }

    fn verdict(check: &dyn Check, c: &FakeCtx) -> Verdict {
        block_on(check.run(c)).unwrap()
    }

    #[test]
    fn the_rows_are_the_apps_then_its_sessions() {
        let rows: Vec<(&str, Slot)> = all().iter().map(|c| (c.row().id, c.row().slot)).collect();
        assert_eq!(rows, [("codex.cli", Slot::App), ("codex.auth", Slot::App), ("codex.sessions", Slot::App), ("agents.codex", Slot::Sessions)]);
    }

    #[test]
    fn the_command_is_the_apps_or_the_one_on_path() {
        assert_eq!(block_on(codex_bin(&FakeCtx::new(facts()))).unwrap(), None);
        let mut f = facts();
        f["codexBin"] = json!("/Applications/ChatGPT.app/codex");
        assert_eq!(block_on(codex_bin(&FakeCtx::new(f.clone()))).unwrap(), None, "the app's codex isn't there");
        let app = FakeCtx::new(f.clone()).file("/Applications/ChatGPT.app/codex", "");
        assert_eq!(block_on(codex_bin(&app)).unwrap().as_deref(), Some("/Applications/ChatGPT.app/codex"));
        let cli = FakeCtx::new(f).cli("codex", "/usr/local/bin/codex");
        assert_eq!(block_on(codex_bin(&cli)).unwrap().as_deref(), Some("/usr/local/bin/codex"), "the one on PATH, when the app's is gone");
        let v = verdict(&Cli, &FakeCtx::new(facts()));
        assert_eq!((v.status, v.detail.as_str()), (Status::Info, "not installed; XenoNet shows Codex's threads once it is"));
        let v = verdict(&Cli, &FakeCtx::new(facts()).cli("codex", "/usr/local/bin/codex").answers("/usr/local/bin/codex --version", "codex-cli 0.50.0-alpha.1\n"));
        assert_eq!(v.detail, "version 0.50.0-alpha.1, /usr/local/bin/codex");
    }

    #[test]
    fn the_sign_in() {
        assert_eq!(verdict(&Auth, &FakeCtx::new(facts())).detail, "Codex isn't installed");
        let says = |text: &str| verdict(&Auth, &FakeCtx::new(facts()).cli("codex", "/bin/codex").answers("/bin/codex login status", text));
        assert_eq!(says("Logged in using ChatGPT\n").detail, "signed in using ChatGPT");
        let v = says("Not logged in\n");
        assert_eq!((v.status, v.fix.as_deref()), (Status::Fail, Some("Run codex login.")));
        assert_eq!(says("").detail, "couldn't tell from codex login status");
        assert_eq!(says("Something else\nmore").detail, "Something else");
    }

    #[test]
    fn the_sessions_folder() {
        assert_eq!(verdict(&Sessions, &FakeCtx::new(facts())).detail, "no ~/.codex/sessions yet: no Codex sessions on this machine");
        assert_eq!(verdict(&Sessions, &FakeCtx::new(facts()).dir("/Users/ada/.codex/sessions", &["2026"])).detail, "~/.codex/sessions");
        let v = verdict(&Sessions, &FakeCtx::new(facts()).denied("/Users/ada/.codex/sessions", "EPERM"));
        assert_eq!((v.status, v.detail.as_str()), (Status::Fail, "can't read ~/.codex/sessions (EPERM)"));
        assert_eq!(verdict(&Sessions, &FakeCtx::new(json!({ "home": "/Users/ada" }))).detail, "no undefined yet: no Codex sessions on this machine");
    }
}
