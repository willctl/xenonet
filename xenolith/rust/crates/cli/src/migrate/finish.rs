//! --finish: once nothing of agent-wiki's runs, deletes its program, settings, state and log
//! folders (and ~/.agent-wiki, and copies an app package captured). Its logs hold clipped tool
//! arguments that Forget cannot reach, so they go too. The wiki is never among them. With
//! --move-wiki, the wiki folder is renamed to ~/Xenolith on the same volume, journaled.
//! --update-protocol renames agent-wiki's tool names in an edited PROTOCOL.md.

use super::Out;
use super::detect::{self, AW_SERVICE, Ctx};
use super::edits::{self, fwd};
use super::journal::{self, Journal, Move};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The only folder names --finish deletes: agent-wiki's own, whatever variable pointed elsewhere.
const NAMES: [&str; 4] = ["AgentWiki", "Agent Wiki", "agent-wiki", ".agent-wiki"];

pub fn finish(ctx: &mut Ctx, out: &Out, move_wiki: bool) -> Result<Journal, String> {
    let work = ctx.lay.work.clone();
    let Some(mut j) = Journal::load(&work)? else { return Err("there is no migration to finish: run --run first".into()) };
    if j.state != "done" && j.state != "finished" {
        return Err(format!("the migration is {} : run --resume first", if j.state == "rolled-back" { "rolled back" } else { "not finished" }));
    }
    let waiting: Vec<&String> = j.pending.iter().filter(|p| p.contains("--resume")).collect();
    if !waiting.is_empty() && j.state == "done" {
        return Err(format!("these still wait: {}. Do them and run --resume, then --finish. Nothing was deleted.", waiting.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("; ")));
    }
    ctx.opts.wiki_dir = Some(PathBuf::from(&j.wiki_dir));
    let left = detect::aw_processes(ctx, &ctx.sys.processes());
    if !left.is_empty() {
        let list: Vec<String> = left.iter().map(|p| format!("pid {} ({})", p.pid, p.kind)).collect();
        return Err(format!("agent-wiki still runs: {}. Restart Claude Code and Codex (their sessions started these), then run --finish again. Nothing was deleted.", list.join(", ")));
    }
    if ctx.lay.os == "windows" && detect::win_service(ctx.sys).is_some() {
        return Err(format!(
            "the {AW_SERVICE} service is still registered. Run --resume and approve the UAC prompt (or run `sc delete {AW_SERVICE}` in a terminal opened with \"Run as administrator\"), then --finish."
        ));
    }

    out.step("1", "agent-wiki's folders");
    let lay = &ctx.lay;
    let wiki = PathBuf::from(&j.wiki_dir);
    let mut keep: Vec<PathBuf> =
        vec![wiki.clone(), lay.home.clone(), lay.xen.config_dir.clone(), lay.xen.data_dir.clone(), lay.xen.state_dir.clone(), lay.xen.log_dir.clone(), lay.xen.cache_dir.clone()];
    if let Some(m) = &j.codex_home {
        keep.push(PathBuf::from(&m.to));
    }
    let mut folders: Vec<PathBuf> = j.aw_folders.iter().map(PathBuf::from).collect();
    for (_, p) in lay.aw_folders() {
        if !folders.contains(&p) {
            folders.push(p);
        }
    }
    for d in folders {
        let k = fwd(&d);
        if j.finish.get(&k).is_some_and(|s| s == "removed") || !d.exists() {
            continue;
        }
        let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !NAMES.contains(&name.as_str()) {
            out.warn(&format!("left in place: {k} (an AGENT_WIKI_* variable named it; delete it yourself once you are sure)"));
            j.finish.insert(k, "left".into());
            continue;
        }
        if let Some(p) = keep.iter().find(|p| p.starts_with(&d)) {
            out.warn(&format!("left in place: {k}, because {} is inside it", fwd(p)));
            j.finish.insert(k, "left".into());
            continue;
        }
        std::fs::remove_dir_all(&d).map_err(|e| format!("cannot delete {k}: {e}"))?;
        out.info(&format!("deleted {k}"));
        j.finish.insert(k, "removed".into());
        j.save(&work)?;
    }

    if move_wiki {
        out.step("2", "The wiki to ~/Xenolith");
        move_wiki_folder(ctx, out, &mut j)?;
    }
    j.state = "finished".into();
    j.save(&work)?;
    journal::log(&work, "finished");
    Ok(j)
}

fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
        root(a) == root(b)
    }
}

/// Xenolith's own services and tray, stopped so the wiki folder can be renamed.
fn stop_xenolith(ctx: &Ctx, out: &Out) {
    let sys = ctx.sys;
    let tray = ctx.lay.xen.data_dir.join("tray").join(ctx.lay.exe("xenolith-tray"));
    if tray.exists() {
        let ini = ctx.lay.xen.data_dir.join("tray").join("xenolith-tray.ini");
        sys.run(&tray, &["--quit", "--config", &ini.to_string_lossy()], &[]);
        out.info("the Xenolith tray quit");
    }
    match ctx.lay.os.as_str() {
        "macos" | "linux" => {
            #[cfg(unix)]
            {
                use crate::install::posix::{CURATOR_LABEL, CURATOR_UNIT, SERVER_LABEL, SERVER_UNIT};
                if ctx.lay.os == "macos" {
                    let domain = format!("gui/{}", sys.uid());
                    for l in [SERVER_LABEL, CURATOR_LABEL] {
                        let plist = ctx.lay.launch_agents.join(format!("{l}.plist"));
                        if plist.exists() {
                            sys.run(Path::new("launchctl"), &["bootout", &domain, &plist.to_string_lossy()], &[]);
                            out.info(&format!("{l}: stopped"));
                        }
                    }
                } else {
                    for u in [SERVER_UNIT, CURATOR_UNIT] {
                        sys.run(Path::new("systemctl"), &["--user", "stop", u], &[]);
                    }
                    out.info("Xenolith's services stopped");
                }
            }
        }
        _ => {
            #[cfg(windows)]
            {
                sys.run(Path::new("sc"), &["stop", crate::install::windows::SERVICE_NAME], &[]);
                out.info("the Xenolith service stopped");
            }
        }
    }
}

fn move_wiki_folder(ctx: &Ctx, out: &Out, j: &mut Journal) -> Result<(), String> {
    let work = ctx.lay.work.clone();
    let from = PathBuf::from(&j.wiki_dir);
    let to = ctx.lay.xen_default_wiki.clone();
    if j.wiki_move.as_ref().is_some_and(|m| m.status == "done") || from == to {
        out.info(&format!("the wiki is at {}", fwd(&to)));
        return Ok(());
    }
    let moved = j.wiki_move.as_ref().is_some_and(|m| m.status == "moved") || (!from.exists() && to.join("PROTOCOL.md").exists());
    if !moved {
        if std::fs::symlink_metadata(&from).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{} is a link; move the folder it points to yourself, then run xenolith install --wiki-dir <new place>", fwd(&from)));
        }
        if to.exists() && std::fs::read_dir(&to).is_ok_and(|mut rd| rd.next().is_some()) {
            return Err(format!("{} already exists and is not empty; the wiki stays at {}", fwd(&to), fwd(&from)));
        }
        if !same_volume(&from, to.parent().unwrap_or(&ctx.lay.home)) {
            return Err(format!("{} is on another volume than {}; a move there would be a copy. The wiki stays where it is.", fwd(&from), fwd(&to)));
        }
        stop_xenolith(ctx, out);
        if !super::run::wait(ctx, || detect::locks(&from).iter().all(|l| !l.live)) {
            return Err("the wiki is still in use (a lock is held); close what holds it, then run --finish --move-wiki again".into());
        }
        j.wiki_move = Some(Move { from: fwd(&from), to: fwd(&to), status: "moving".into() });
        j.save(&work)?;
        if to.exists() {
            let _ = std::fs::remove_dir(&to);
        }
        std::fs::rename(&from, &to).map_err(|e| format!("cannot move {} to {}: {e}", fwd(&from), fwd(&to)))?;
        out.info(&format!("moved {} -> {}", fwd(&from), fwd(&to)));
    }
    j.wiki_move = Some(Move { from: fwd(&from), to: fwd(&to), status: "moved".into() });
    j.save(&work)?;
    // Xenolith's config, the tray's settings, the services and (Windows) the folder grants follow.
    let cfg = ctx.lay.xen.config_file();
    edits::edit(&cfg, |t| {
        let mut v = edits::parse_json(t, &cfg)?;
        v["wikiDir"] = serde_json::json!(fwd(&to));
        Ok(Some(edits::to_json(&v)))
    })?;
    let mut args = vec!["install".to_string(), "--wiki-dir".into(), fwd(&to)];
    if let Some(p) = ctx.opts.port {
        args.extend(["--port".into(), p.to_string()]);
    }
    if let Some(f) = &ctx.opts.from {
        args.extend(["--from".into(), f.to_string_lossy().into_owned()]);
    }
    let r = ctx.sys.xenolith(&args);
    if !r.ok {
        return Err(format!("the wiki moved to {}, but xenolith install failed: {}. Run xenolith install --wiki-dir {} to finish.", fwd(&to), r.last_line(), fwd(&to)));
    }
    out.info("xenolith install: the service, the tray and the apps use the new folder");
    j.wiki_move.as_mut().unwrap().status = "done".into();
    j.save(&work)
}

static OLD_TOOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bwiki_(start|search|read|log|upsert_page)\b").unwrap());

/// Renames agent-wiki's tool names (wiki_*) to Xenolith's (context_*) in PROTOCOL.md, keeping the
/// old text in .history/.
pub fn update_protocol(wiki: &Path, out: &Out) -> Result<bool, String> {
    let file = wiki.join("PROTOCOL.md");
    let Some(raw) = edits::read(&file)? else { return Err(format!("{} does not exist", fwd(&file))) };
    if !OLD_TOOL.is_match(&raw) {
        out.say(format!("{} names no old tools; nothing to rename.", fwd(&file)));
        return Ok(false);
    }
    let hist = wiki.join(".history").join(format!("PROTOCOL-{}.md", xenolith_core::text::history_stamp(&xenolith_core::text::now())));
    xenolith_core::wiki::write_if_missing(&hist, &raw).map_err(|e| format!("cannot keep a copy in {}: {e}", fwd(&hist)))?;
    let n = OLD_TOOL.find_iter(&raw).count();
    edits::edit(&file, |t| {
        Ok(Some(
            OLD_TOOL
                .replace_all(t, |c: &regex::Captures| {
                    let old = &c[0];
                    crate::mcp::OLD_NAMES.iter().find(|(o, _)| *o == old).map(|(_, new)| new.to_string()).unwrap_or_else(|| old.to_string())
                })
                .into_owned(),
        ))
    })?;
    out.say(format!("{}: renamed {n} mention(s) of agent-wiki's tools to Xenolith's; the old text is in {}", fwd(&file), fwd(&hist)));
    Ok(true)
}
