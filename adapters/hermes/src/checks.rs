//! Hermes Agent's doctor check: whether XenoNet's plugin for Hermes is
//! installed, turned on, has its token, and has been heard from lately.

use crate::plugin as hp;
use async_trait::async_trait;
use xenonet_adapter::check::Facts;
use xenonet_adapter::{Check, CheckCtx, CheckRow, Slot, Stop, Verdict};
use xenonet_base::text::js_string;
use xenonet_base::units::duration;

/// The checks, in the order they show.
pub fn all() -> Vec<Box<dyn Check>> {
    vec![Box::new(Plugin)]
}

struct Plugin;

#[async_trait(?Send)]
impl Check for Plugin {
    fn row(&self) -> CheckRow {
        CheckRow { id: "actors.hermes-plugin", group: "Actors", title: "XenoNet's plugin for Hermes reports", slot: Slot::Actors }
    }

    /// It reads files, not XenoNet, so it answers with actors off too.
    async fn run(&self, c: &dyn CheckCtx) -> Result<Verdict, Stop> {
        let f = Facts(c.facts());
        let (home, given) = (f.home(), f.get("hermesHome").map(js_string));
        if !c.exists(&hp::root(&home, given.as_deref()).display().to_string()).await? {
            return Ok(Verdict::ok("Hermes Agent isn't on this computer, so its plugin has nothing to do"));
        }
        let now = c.now().await?;
        let st = hp::inspect(&home, given.as_deref(), now);
        let shown = |p: &std::path::Path| xenonet_base::paths::tilde(&p.display().to_string(), &home);
        if !st.installed {
            return Ok(Verdict::info("XenoNet's plugin for Hermes isn't installed, so Hermes is only observed", Some("xenonet connect hermes adds it. It's optional.")));
        }
        if !st.current {
            return Ok(Verdict::warn("the plugin in Hermes's plugins folder isn't the one this XenoNet ships", Some("xenonet connect hermes updates it.")));
        }
        if st.token == "loose" {
            return Ok(Verdict::warn("the plugin's token file can be read by other users, so the plugin ignores it", Some(&format!("chmod 600 {}", shown(&st.dir.join("token"))))));
        }
        let todo = hp::owner_steps(&st, &shown);
        if !todo.is_empty() {
            let lacks: Vec<&str> = [(st.token != "ok").then_some("has no token"), (!st.enabled).then_some("isn't turned on")].into_iter().flatten().collect();
            return Ok(Verdict::info(format!("the plugin is installed but {}", lacks.join(" and ")), Some(&todo.join(" "))));
        }
        Ok(match st.reported {
            None => Verdict::info("the plugin is installed, turned on and has its token, but hasn't reported yet", Some("It starts with Hermes's next session.")),
            Some((false, _)) => Verdict::warn(
                "the plugin can't reach XenoNet or isn't accepted: its last heartbeat failed",
                Some("Check the token in XenoNet's Actors panel (New token), and that XenoNet is running."),
            ),
            Some((true, age)) if age > hp::STALE_MS => Verdict::info(format!("the plugin last reported {} ago, so no Hermes process is running it now", duration(Some(age))), None),
            Some((true, age)) => Verdict::ok(format!("the plugin is installed, turned on and reporting (last heartbeat {} ago)", duration(Some(age)))),
        })
    }
}
