//! Grok Build's doctor checks: the command, its sign-in, the sessions
//! folder, whether it runs XenoNet's hooks, and whether its sessions are
//! connected.

use async_trait::async_trait;
use serde_json::Value;
use xenonet_adapter::check::{connected, folder, list, privacy_fix, s, Facts, Folder};
use xenonet_adapter::{Check, CheckCtx, CheckRow, Slot, Stop, Verdict};
use xenonet_os::nodepath::style_of;
use xenonet_text::text::{display, truthy};
use xenonet_text::units::plural;

/// The checks, in the order they show.
pub fn all() -> Vec<Box<dyn Check>> {
    vec![Box::new(Cli), Box::new(Auth), Box::new(Sessions), Box::new(Hooks), Box::new(Connected)]
}

const GROUP: &str = "Grok Build";

struct Cli;
struct Auth;
struct Sessions;
struct Hooks;
struct Connected;

/// Grok Build's folder (GROK_HOME's, or ~/.grok) and a path in it.
fn grok_home(f: Facts<'_>) -> String {
    f.get("paths").and_then(|p| p.get("grokHome")).filter(|v| !v.is_null()).map_or_else(
        || {
            let home = f.home();
            style_of(&home).join(&[&home, ".grok"])
        },
        display,
    )
}
fn in_grok(f: Facts<'_>, parts: &[&str]) -> String {
    let home = grok_home(f);
    let mut all = vec![home.as_str()];
    all.extend_from_slice(parts);
    style_of(&home).join(&all)
}

/// Grok Build's command: grok on PATH, or where its installer puts it,
/// which a shell's PATH may not have yet.
async fn grok_bin(c: &dyn CheckCtx) -> Result<Option<String>, Stop> {
    if let Some(b) = c.which("grok").await? {
        return Ok(Some(b));
    }
    let f = Facts(c.facts());
    let own = in_grok(f, &["bin", if f.platform() == "win32" { "grok.exe" } else { "grok" }]);
    Ok(c.exists(&own).await?.then_some(own))
}

/// Whether a settings file lets Grok Build run only the hooks an
/// organization sets.
fn managed_hooks_only(text: &str) -> bool {
    re!(r"(?m)^\s*(allow_managed_hooks_only|allowManagedHooksOnly)\s*=\s*true\b").is_match(text)
}

#[async_trait(?Send)]
impl Check for Cli {
    fn row(&self) -> CheckRow {
        CheckRow { id: "grok.cli", group: GROUP, title: "Grok Build is installed", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let Some(bin) = grok_bin(c).await? else {
            return Ok(Verdict::info("not installed; XenoNet shows Grok Build's sessions once it is", Some("To use Grok Build, install it (x.ai/cli), then run grok login.")));
        };
        let v = c.run(&bin, &["--version"], 4000.0).await?;
        let version = re!(r"\d+\.\d+\.\d+[A-Za-z0-9_.-]*").find(&s(v.get("stdout"))).map(|m| m.as_str().to_string());
        Ok(Verdict::ok(format!("{}{}", version.map_or(String::new(), |v| format!("version {v}, ")), c.tilde(&bin))))
    }
}

#[async_trait(?Send)]
impl Check for Auth {
    fn row(&self) -> CheckRow {
        CheckRow { id: "grok.auth", group: GROUP, title: "Grok Build is signed in", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        if grok_bin(c).await?.is_none() {
            return Ok(Verdict::skip("Grok Build isn't installed"));
        }
        let f = Facts(c.facts());
        // Whether its sign-in is there, never what's in it: auth.json holds
        // the tokens.
        if c.exists(&in_grok(f, &["auth.json"])).await? {
            return Ok(Verdict::ok("signed in"));
        }
        if truthy(c.facts().get("grokKeySet")) {
            return Ok(Verdict::ok("with an API key (XAI_API_KEY)"));
        }
        Ok(Verdict::info("not signed in yet", Some("Run grok login.")))
    }
}

#[async_trait(?Send)]
impl Check for Sessions {
    fn row(&self) -> CheckRow {
        CheckRow { id: "grok.sessions", group: GROUP, title: "XenoNet can read Grok Build's sessions", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let dir = in_grok(Facts(c.facts()), &["sessions"]);
        let shown = c.tilde(&dir);
        Ok(match folder(c, Some(&dir)).await? {
            Folder::Missing => Verdict::info(format!("no {shown} yet: no Grok Build sessions on this machine"), None),
            Folder::Denied(code) => Verdict::fail(format!("can't read {shown} ({code})"), Some(&privacy_fix(c, &shown))),
            Folder::Ok(count) => Verdict::ok(format!("{} of sessions in {shown}", plural(count as f64, "folder", None))),
        })
    }
}

#[async_trait(?Send)]
impl Check for Hooks {
    fn row(&self) -> CheckRow {
        CheckRow { id: "grok.hooks", group: GROUP, title: "Grok Build runs XenoNet's hooks", slot: Slot::App }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let f = Facts(c.facts());
        let a = f
            .get("connect")
            .filter(|v| truthy(Some(v)))
            .and_then(|cn| cn.get("apps"))
            .and_then(Value::as_array)
            .and_then(|apps| apps.iter().find(|x| x.get("app").and_then(Value::as_str) == Some("grok")));
        let Some(a) = a else { return Ok(Verdict::skip("needs XenoNet running")) };
        let hooks = list(a.get("changes")).into_iter().find(|x| x.get("id").and_then(Value::as_str) == Some("hooks"));
        if hooks.and_then(|h| h.get("state").and_then(Value::as_str).map(String::from)).as_deref() != Some("present") {
            return Ok(Verdict::skip("XenoNet's hooks aren't in Grok Build's settings"));
        }
        // An organization can let Grok run only the hooks it sets
        // (allow_managed_hooks_only), in its own files or the ones it
        // syncs.
        for file in
            ["/etc/grok/requirements.toml".to_string(), "/etc/grok/managed_config.toml".to_string(), in_grok(f, &["requirements.toml"]), in_grok(f, &["managed_config.toml"])]
        {
            if managed_hooks_only(&c.read_text(&file).await) {
                return Ok(Verdict::warn(
                    format!("Grok Build runs only the hooks your organization sets ({}), so XenoNet's don't run", c.tilde(&file)),
                    Some("Ask whoever manages Grok Build here to allow them. XenoNet's tools still work without them."),
                ));
            }
        }
        // Hooks you turned off in Grok's /hooks.
        if c.read_text(&in_grok(f, &["disabled-hooks"])).await.to_ascii_lowercase().contains("xenonet") {
            return Ok(Verdict::warn("XenoNet's hooks are turned off in Grok Build", Some("Run /hooks in Grok Build, select XenoNet's hooks and press Space to turn them on.")));
        }
        Ok(Verdict::ok(format!("in {}, which Grok Build runs without asking", c.tilde(&in_grok(f, &["hooks"])))))
    }
}

#[async_trait(?Send)]
impl Check for Connected {
    fn row(&self) -> CheckRow {
        CheckRow { id: "agents.grok", group: "Sessions", title: "Grok Build sessions are connected to XenoNet", slot: Slot::Sessions }
    }
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        Ok(connected(c, "grok", "Grok Build"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xenonet_adapter::check::fake::{block_on, FakeCtx};
    use xenonet_adapter::check::Status;

    fn facts() -> Value {
        json!({ "home": "/Users/ada", "platform": "darwin", "paths": {} })
    }

    fn with_hooks(state: &str) -> Value {
        let mut f = facts();
        f["connect"] = json!({ "port": 47840, "apps": [{ "app": "grok", "installed": true, "changes": [{ "id": "hooks", "state": state }] }] });
        f
    }

    fn verdict(check: &dyn Check, c: &FakeCtx) -> Verdict {
        block_on(check.run(c)).unwrap()
    }

    #[test]
    fn the_rows_are_the_apps_then_its_sessions() {
        let rows: Vec<(&str, Slot)> = all().iter().map(|c| (c.row().id, c.row().slot)).collect();
        assert_eq!(rows, [("grok.cli", Slot::App), ("grok.auth", Slot::App), ("grok.sessions", Slot::App), ("grok.hooks", Slot::App), ("agents.grok", Slot::Sessions)]);
    }

    #[test]
    fn the_folder_is_grok_home_or_dot_grok() {
        assert_eq!(in_grok(Facts(&facts()), &["sessions"]), "/Users/ada/.grok/sessions");
        let f = json!({ "home": "/Users/ada", "paths": { "grokHome": "/opt/grok" } });
        assert_eq!(in_grok(Facts(&f), &["auth.json"]), "/opt/grok/auth.json");
        let win = json!({ "home": "C:\\Users\\ada", "platform": "win32", "paths": {} });
        assert_eq!(in_grok(Facts(&win), &["bin", "grok.exe"]), "C:\\Users\\ada\\.grok\\bin\\grok.exe");
    }

    #[test]
    fn the_command_and_the_sign_in() {
        let none = FakeCtx::new(facts());
        let v = verdict(&Cli, &none);
        assert_eq!((v.status, v.fix.as_deref()), (Status::Info, Some("To use Grok Build, install it (x.ai/cli), then run grok login.")));
        assert_eq!(verdict(&Auth, &none).detail, "Grok Build isn't installed");
        // The installer's copy counts before the shell's PATH has it.
        let own = FakeCtx::new(facts()).file("/Users/ada/.grok/bin/grok", "").answers("/Users/ada/.grok/bin/grok --version", "grok 1.2.3\n");
        assert_eq!(verdict(&Cli, &own).detail, "version 1.2.3, ~/.grok/bin/grok");
        let v = verdict(&Auth, &own);
        assert_eq!((v.status, v.detail.as_str(), v.fix.as_deref()), (Status::Info, "not signed in yet", Some("Run grok login.")));
        assert_eq!(verdict(&Auth, &FakeCtx::new(facts()).cli("grok", "/bin/grok").file("/Users/ada/.grok/auth.json", "{}")).detail, "signed in");
        let mut keyed = facts();
        keyed["grokKeySet"] = json!(true);
        assert_eq!(verdict(&Auth, &FakeCtx::new(keyed).cli("grok", "/bin/grok")).detail, "with an API key (XAI_API_KEY)");
    }

    #[test]
    fn the_sessions_folder() {
        assert_eq!(verdict(&Sessions, &FakeCtx::new(facts())).detail, "no ~/.grok/sessions yet: no Grok Build sessions on this machine");
        assert_eq!(verdict(&Sessions, &FakeCtx::new(facts()).dir("/Users/ada/.grok/sessions", &["a"])).detail, "1 folder of sessions in ~/.grok/sessions");
        let v = verdict(&Sessions, &FakeCtx::new(facts()).denied("/Users/ada/.grok/sessions", "EACCES"));
        assert_eq!((v.status, v.detail.as_str()), (Status::Fail, "can't read ~/.grok/sessions (EACCES)"));
    }

    #[test]
    fn the_hooks_run_unless_managed_or_turned_off() {
        assert_eq!(verdict(&Hooks, &FakeCtx::new(facts())).detail, "needs XenoNet running");
        assert_eq!(verdict(&Hooks, &FakeCtx::new(with_hooks("missing"))).detail, "XenoNet's hooks aren't in Grok Build's settings");
        assert_eq!(verdict(&Hooks, &FakeCtx::new(with_hooks("present"))).detail, "in ~/.grok/hooks, which Grok Build runs without asking");
        let managed = FakeCtx::new(with_hooks("present")).file("/Users/ada/.grok/managed_config.toml", "allow_managed_hooks_only = true\n");
        let v = verdict(&Hooks, &managed);
        assert_eq!((v.status, v.detail.as_str()), (Status::Warn, "Grok Build runs only the hooks your organization sets (~/.grok/managed_config.toml), so XenoNet's don't run"));
        assert!(!managed_hooks_only("# allow_managed_hooks_only = true"), "a comment doesn't count");
        let off = FakeCtx::new(with_hooks("present")).file("/Users/ada/.grok/disabled-hooks", "XenoNet: stop\n");
        assert_eq!(verdict(&Hooks, &off).detail, "XenoNet's hooks are turned off in Grok Build");
    }
}
