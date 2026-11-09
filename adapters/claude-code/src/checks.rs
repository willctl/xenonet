//! Claude Code's doctor checks: the command, its sign-in, the transcripts
//! folder, the plan usage XenoNet reads, and whether its sessions are
//! connected.

use async_trait::async_trait;
use serde_json::{json, Value};
use xenonet_adapter::check::{connected, folder, privacy_fix, s, Facts, Folder};
use xenonet_adapter::{Check, CheckCtx, CheckRow, Slot, Stop, Verdict};
use xenonet_base::text::{js_string, truthy};
use xenonet_base::units::plural;

/// The checks, in the order they show.
pub fn all() -> Vec<Box<dyn Check>> {
    vec![Box::new(Cli), Box::new(Auth), Box::new(Transcripts), Box::new(Usage), Box::new(Sessions)]
}

const GROUP: &str = "Claude Code";

struct Cli;
struct Auth;
struct Transcripts;
struct Usage;
struct Sessions;

#[async_trait(?Send)]
impl Check for Cli {
    fn row(&self) -> CheckRow {
        CheckRow { id: "claude.cli", group: GROUP, title: "Claude Code is installed", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let Some(bin) = c.which("claude").await? else {
            return Ok(Verdict::warn(
                "the claude command isn't on your PATH, and handing off to Claude and connecting Claude Code need it",
                Some("Install Claude Code: https://claude.com/claude-code"),
            ));
        };
        let v = c.run(&bin, &["--version"], 4000.0).await?;
        let version = re!(r"\d+\.\d+\.\d+").find(&s(v.get("stdout"))).map(|m| m.as_str().to_string());
        Ok(Verdict::ok(format!("{}{}", version.map_or(String::new(), |v| format!("version {v}, ")), c.tilde(&bin))))
    }
}

#[async_trait(?Send)]
impl Check for Auth {
    fn row(&self) -> CheckRow {
        CheckRow { id: "claude.auth", group: GROUP, title: "Claude Code is signed in", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let Some(bin) = c.which("claude").await? else { return Ok(Verdict::skip("Claude Code isn't installed")) };
        let r = c.run(&bin, &["auth", "status"], 6000.0).await?;
        if truthy(r.get("timedOut")) {
            return Ok(Verdict::warn("claude auth status didn't answer in 6 seconds", None));
        }
        let st = xenonet_base::json::parse(&s(r.get("stdout"))).ok();
        if let Some(st) = &st {
            if truthy(st.get("loggedIn")) {
                let by = st.get("authMethod").filter(|v| truthy(Some(v))).map_or(String::new(), |m| format!(" with {}", js_string(m)));
                return Ok(Verdict::ok(format!("signed in{by}")));
            }
            if truthy(Some(st)) && st.get("loggedIn") == Some(&json!(false)) {
                return Ok(Verdict::fail("not signed in: XenoNet can't read Claude plan usage, and new sessions can't start", Some("Run claude and sign in with /login.")));
            }
        }
        Ok(Verdict::warn("couldn't tell from claude auth status", Some("Run claude auth status to see why.")))
    }
}

#[async_trait(?Send)]
impl Check for Transcripts {
    fn row(&self) -> CheckRow {
        CheckRow { id: "claude.transcripts", group: GROUP, title: "XenoNet can read Claude Code's sessions", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let dir = Facts(c.facts()).path("claudeProjectsDir");
        let shown = c.tilde(&dir.clone().unwrap_or_else(|| "undefined".into()));
        Ok(match folder(c, dir.as_deref()).await? {
            Folder::Missing => Verdict::info(format!("no {shown} yet: no Claude Code sessions on this machine"), None),
            Folder::Denied(code) => Verdict::fail(format!("can't read {shown} ({code})"), Some(&privacy_fix(c, &shown))),
            Folder::Ok(count) => Verdict::ok(format!("{} of transcripts in {shown}", plural(count as f64, "project", None))),
        })
    }
}

#[async_trait(?Send)]
impl Check for Usage {
    fn row(&self) -> CheckRow {
        CheckRow { id: "claude.usage", group: GROUP, title: "Claude plan usage is current", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let f = Facts(c.facts());
        let Some(live) = f.get("live").filter(|v| truthy(Some(v))) else { return Ok(Verdict::skip("needs XenoNet running")) };
        let Some(cl) = live.get("limits").and_then(|l| l.get("claude")).filter(|v| truthy(Some(v))) else { return Ok(Verdict::info("no plan usage reading yet", None)) };
        if !truthy(cl.get("stale")) {
            return Ok(Verdict::ok(format!("read {}", if cl.get("source").and_then(Value::as_str) == Some("anthropic") { "from Anthropic" } else { "from Claude desktop" })));
        }
        let since = match cl.get("observedAt").filter(|v| truthy(Some(v))) {
            Some(at) => format!("no reading since {}", c.local_time(at).await?),
            None => "no reading yet".into(),
        };
        let reason = cl.get("reason").filter(|v| !v.is_null()).map_or_else(String::new, js_string);
        let any = |words: &[&str]| words.iter().any(|w| reason.contains(w));
        if any(&["expired", "refused"]) {
            return Ok(Verdict::warn(format!("{reason}, so there's {since}"), Some("Open Claude Code once; it refreshes its sign-in, and XenoNet reads it on its next try.")));
        }
        if any(&["asked to wait", "couldn't reach", "no Claude Code sign-in"]) {
            return Ok(Verdict::info(format!("{reason}, so there's {since}. XenoNet tries again by itself."), None));
        }
        Ok(Verdict::warn(if truthy(cl.get("reason")) { format!("{reason}, so there's {since}") } else { format!("there's {since}") }, None))
    }
}

#[async_trait(?Send)]
impl Check for Sessions {
    fn row(&self) -> CheckRow {
        CheckRow { id: "agents.claude", group: "Sessions", title: "Claude Code sessions are connected to XenoNet", slot: Slot::Sessions }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        Ok(connected(c, "claude", "Claude Code"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::check::fake::{block_on, FakeCtx};
    use xenonet_adapter::check::Status;

    fn facts() -> Value {
        json!({ "home": "/Users/ada", "platform": "darwin", "paths": { "claudeProjectsDir": "/Users/ada/.claude/projects" } })
    }

    fn verdict(check: &dyn Check, c: &FakeCtx) -> Verdict {
        block_on(check.run(c)).unwrap()
    }

    #[test]
    fn the_rows_are_the_apps_then_its_sessions() {
        let rows: Vec<(&str, Slot)> = all().iter().map(|c| (c.row().id, c.row().slot)).collect();
        assert_eq!(
            rows,
            [("claude.cli", Slot::App), ("claude.auth", Slot::App), ("claude.transcripts", Slot::App), ("claude.usage", Slot::App), ("agents.claude", Slot::Sessions)]
        );
    }

    #[test]
    fn the_command_and_its_sign_in() {
        let none = FakeCtx::new(facts());
        let v = verdict(&Cli, &none);
        assert_eq!((v.status, v.fix.as_deref()), (Status::Warn, Some("Install Claude Code: https://claude.com/claude-code")));
        assert_eq!(verdict(&Auth, &none).detail, "Claude Code isn't installed");
        let installed = FakeCtx::new(facts())
            .cli("claude", "/Users/ada/.local/bin/claude")
            .answers("/Users/ada/.local/bin/claude --version", "2.1.0 (Claude Code)\n")
            .answers("/Users/ada/.local/bin/claude auth status", r#"{"loggedIn":true,"authMethod":"claude.ai"}"#);
        assert_eq!(verdict(&Cli, &installed).detail, "version 2.1.0, ~/.local/bin/claude");
        assert_eq!(verdict(&Auth, &installed).detail, "signed in with claude.ai");
        let out = FakeCtx::new(facts()).cli("claude", "/bin/claude").answers("/bin/claude auth status", r#"{"loggedIn":false}"#);
        let v = verdict(&Auth, &out);
        assert_eq!((v.status, v.fix.as_deref()), (Status::Fail, Some("Run claude and sign in with /login.")));
        let v = verdict(&Auth, &FakeCtx::new(facts()).cli("claude", "/bin/claude").answers("/bin/claude auth status", "not json"));
        assert_eq!((v.status, v.detail.as_str()), (Status::Warn, "couldn't tell from claude auth status"));
    }

    #[test]
    fn the_transcripts_folder() {
        assert_eq!(verdict(&Transcripts, &FakeCtx::new(facts())).detail, "no ~/.claude/projects yet: no Claude Code sessions on this machine");
        let two = FakeCtx::new(facts()).dir("/Users/ada/.claude/projects", &["-a", "-b"]);
        assert_eq!(verdict(&Transcripts, &two).detail, "2 projects of transcripts in ~/.claude/projects");
        let v = verdict(&Transcripts, &FakeCtx::new(facts()).denied("/Users/ada/.claude/projects", "EACCES"));
        assert_eq!((v.status, v.detail.as_str()), (Status::Fail, "can't read ~/.claude/projects (EACCES)"));
        assert!(v.fix.unwrap().starts_with("Let XenoNet read ~/.claude/projects"));
    }

    #[test]
    fn plan_usage() {
        assert_eq!(verdict(&Usage, &FakeCtx::new(facts())).status, Status::Skip);
        let live = |claude: Value| {
            let mut f = facts();
            f["live"] = json!({ "limits": { "claude": claude } });
            FakeCtx::new(f)
        };
        assert_eq!(verdict(&Usage, &live(json!({ "stale": false, "source": "anthropic" }))).detail, "read from Anthropic");
        let v = verdict(&Usage, &live(json!({ "stale": true, "reason": "the sign-in expired", "observedAt": 1700000000000.0 })));
        assert_eq!((v.status, v.detail.as_str()), (Status::Warn, "the sign-in expired, so there's no reading since local time 1700000000000"));
        let v = verdict(&Usage, &live(json!({ "stale": true, "reason": "Anthropic asked to wait" })));
        assert_eq!((v.status, v.detail.as_str()), (Status::Info, "Anthropic asked to wait, so there's no reading yet. XenoNet tries again by itself."));
        assert_eq!(verdict(&Usage, &live(json!({ "stale": true }))).detail, "there's no reading yet");
    }
}
