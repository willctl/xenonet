//! What each command does. Every function returns what happened as JSON;
//! `lib.rs` prints it, as JSON or as a sentence.

use crate::args::{FacetArg, Settle};
use crate::browser;
use crate::cells::Cells;
use crate::error::{CliError, Result};
use crate::home::{read_holder, Home};
use adytum_exec::{Command, Runner, SystemRunner};
use eidolon_browser::BrowserFacet;
use eidolon_facet::{abort_at, open_facets, seal_facets, Facet, FolderFacet};
use eidolon_model::{EidolonId, EidolonSpec, FacetId, FacetKind, FacetSpec, Holder, SignedSeat, State, Wall};
use eidolon_move::{receive, seat_first, send, Holding};
use eidolon_pack::{load, seal, FacetManifest, Manifest, Node, SignedHead, FORMAT};
use eidolon_policy::{leave_out_is_inside, may_hold, may_open, same_wall};
use eidolon_vault::{ChunkName, EidolonKey, KeyRing, Keys, Vault};
use eidolon_view::{OpenOptions, View, ViewGuard};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

/// Everything a command needs from the machine it runs on.
pub struct Ctx {
    pub home: Home,
    pub cells: Option<Box<dyn Cells>>,
}

impl Ctx {
    fn cells(&self) -> Result<&dyn Cells> {
        self.cells.as_deref().ok_or_else(|| CliError::failed("no_cells", "This command needs cells.", "Set up Adytum first."))
    }
}

fn secs(d: Duration) -> f64 {
    (d.as_secs_f64() * 1000.0).round() / 1000.0
}

fn holding_of(ctx: &Ctx, eid: &EidolonId) -> Result<Holding> {
    let root = ctx.home.holdings();
    if !Holding::exists(&root, eid) {
        return Err(CliError::failed("unknown_eidolon", format!("This machine holds no eidolon {eid}."), "Run `eidolon ls` to see what it holds."));
    }
    Ok(Holding::load(&root, eid)?)
}

/// The spec's facets. With `sealing`, a browser is not started again after
/// it was taken: a sealed eidolon is open nowhere.
fn facets_of(spec: &EidolonSpec, sealing: bool) -> Result<Vec<Box<dyn Facet>>> {
    spec.facets
        .iter()
        .map(|f| match f.kind {
            FacetKind::Folder => Ok(Box::new(FolderFacet::new(f.clone())?) as Box<dyn Facet>),
            FacetKind::Browser => {
                let facet: Box<dyn Facet> = Box::new(BrowserFacet::new(f.clone())?);
                Ok(if sealing { Box::new(browser::StaysStopped(facet)) as Box<dyn Facet> } else { facet })
            }
        })
        .collect()
}

/// An open that began and did not finish is not a seat to seal or move from.
fn unfinished(eid: &EidolonId, cell: &str) -> CliError {
    CliError::failed(
        "open_unfinished",
        format!("An open of that eidolon began in {cell} and did not finish, so the files there may be partial."),
        format!("Run `eidolon open {eid} --cell {cell}` to finish it, or open it in another cell and {cell} is cleared first."),
    )
}

fn keys_for(ring: &KeyRing<'_>, eid: &EidolonId) -> Result<Keys> {
    let key = ring
        .get(&eid.to_string())?
        .ok_or_else(|| CliError::failed("no_key", "This machine has no key for that eidolon.", "It was moved away, or never lived here; ask the machine that holds it."))?;
    Ok(Keys::new(&key)?)
}

fn head_of(holding: &Holding) -> Result<SignedHead> {
    serde_json::from_slice(&holding.head).map_err(|_| CliError::failed("bad_head", "The holding's head is damaged.", "Restore it from the machine that sealed the eidolon."))
}

/// The head must be signed by a machine on the holder list. (The seat names
/// who holds the eidolon now; the head was signed by whoever last sealed
/// it, which after several moves may be neither the holder nor the one
/// before.)
fn check_head(spec: &EidolonSpec, head: &SignedHead) -> Result<()> {
    if spec.holders.iter().any(|h| head.verify(&h.sign)) {
        Ok(())
    } else {
        Err(CliError::failed("bad_head_signature", "The head is not signed by any machine on the holder list.", "Do not open it; ask the machine that sealed it to seal again."))
    }
}

/// Every chunk the head needs, by name.
fn chunks_of(head: &SignedHead, manifest: &Manifest) -> Vec<ChunkName> {
    let mut names: BTreeSet<ChunkName> = head.head.manifest.iter().copied().collect();
    for facet in &manifest.facets {
        for entry in &facet.entries {
            if let Node::File { chunks, .. } = &entry.node {
                names.extend(chunks.iter().copied());
            }
        }
    }
    names.into_iter().collect()
}

/// Opens every chunk the head needs (read from the head itself, not from the
/// holding's list, which a kill between two writes can leave behind); a chunk
/// that is missing or does not match its name is a `changed_chunk`.
fn check_chunks(head: &SignedHead, keys: &Keys, vault: &Vault) -> Result<usize> {
    let names = chunks_of(head, &load(head, keys, vault)?);
    let mut bad = 0;
    for name in &names {
        if vault.get(name).and_then(|sealed| keys.open(name, &sealed)).is_err() {
            bad += 1;
        }
    }
    if bad > 0 {
        return Err(CliError::failed(
            "changed_chunk",
            format!("{bad} of {} chunks did not open or did not match their names.", names.len()),
            "Do not open this copy; move the eidolon again from the machine that sealed it.",
        ));
    }
    Ok(names.len())
}

fn file_count(manifest: &Manifest) -> usize {
    manifest.facets.iter().flat_map(|f| &f.entries).filter(|e| matches!(e.node, Node::File { .. })).count()
}

// ---- machine ----------------------------------------------------------------

pub fn machine_init(ctx: &Ctx, name: &str, key_store: &str) -> Result<Value> {
    match key_store {
        "auto" | "file" => {}
        "os" => {
            return Err(CliError::failed(
                "no_os_key_store",
                "This build has no operating-system key store it can use from here.",
                "Use `--key-store file`, or leave the option out.",
            ))
        }
        other => return Err(CliError::usage(format!("{other:?} is not a key store."), "Use `file`.")),
    }
    let store = ctx.home.store()?;
    let machine = ctx.home.init(name, &store)?;
    Ok(json!({ "ok": true, "holder": machine.holder, "key_store": machine.key_store, "warning": crate::home::FILE_STORE_WARNING }))
}

pub fn machine_show(ctx: &Ctx) -> Result<Holder> {
    let store = ctx.home.store()?;
    Ok(ctx.home.machine(&store)?.holder)
}

// ---- new --------------------------------------------------------------------

pub fn new(ctx: &Ctx, wall: &str, holders: &[String], facets: &[FacetArg], stdin: &mut dyn FnMut() -> std::io::Result<Vec<u8>>) -> Result<Value> {
    let store = ctx.home.store()?;
    let machine = ctx.home.machine(&store)?;
    let wall: Wall = wall.parse()?;
    let holders: Vec<Holder> = holders.iter().map(|h| read_holder(h, stdin)).collect::<Result<_>>()?;
    let mut specs = Vec::new();
    for f in facets {
        for pattern in &f.leave_out {
            leave_out_is_inside(pattern)?;
        }
        specs.push(FacetSpec { id: FacetId::new_random()?, kind: f.kind, name: f.name.clone(), path: f.path.clone(), leave_out: f.leave_out.clone() });
    }
    let spec = EidolonSpec { id: EidolonId::new_random()?, wall, holders, facets: specs };
    spec.validate()?;
    may_hold(&spec, &machine.key)?;
    facets_of(&spec, false)?;
    let vault = ctx.home.vault()?;
    let key = EidolonKey::generate()?;
    let keys = Keys::new(&key)?;
    // Sealed from the start, with nothing in it: `seal --cell` fills it.
    let first = Manifest {
        format: FORMAT,
        eidolon: spec.id,
        generation: 1,
        facets: spec.facets.iter().map(|f| FacetManifest { facet: f.id, kind: f.kind, entries: Vec::new() }).collect(),
    };
    let head = seal(&first, &keys, &vault, &machine.signer)?;
    let chunks = chunks_of(&head, &first);
    let seat = seat_first(&spec, &machine.signer)?;
    let id = spec.id;
    KeyRing::new(&store).put(&id.to_string(), &key)?;
    if let Err(e) = Holding::create(&ctx.home.holdings(), spec, seat, serde_json::to_vec(&head)?, chunks) {
        // No holding, so no use for the key.
        let _ = KeyRing::new(&store).delete(&id.to_string());
        return Err(e.into());
    }
    Ok(json!({ "ok": true, "eidolon": id, "generation": 1 }))
}

// ---- seal and open ------------------------------------------------------------

pub async fn seal_cmd(ctx: &Ctx, eid: &EidolonId, cell: Option<&str>) -> Result<Value> {
    let started = Instant::now();
    let store = ctx.home.store()?;
    let machine = ctx.home.machine(&store)?;
    let mut holding = holding_of(ctx, eid)?;
    // Sealing from the cell it is open in, or taking a cell's files into a
    // sealed one. Both need the seat, and neither works on a retired copy.
    let from = match (&holding.state, cell) {
        (State::Opening { cell: unfinished_in }, _) => return Err(unfinished(eid, unfinished_in)),
        (State::Open { cell: open }, Some(given)) if open != given => {
            return Err(CliError::failed("second_seat", format!("That eidolon is open in {open}, not {given}."), format!("Seal it from {open}.")));
        }
        (State::Open { cell: open }, _) => open.clone(),
        (_, Some(given)) => given.to_owned(),
        (_, None) => return Err(CliError::usage("This eidolon is sealed, so it needs --cell to say where to take its files from.", "Add `--cell <cell>`.")),
    };
    let check_state = if matches!(holding.state, State::Open { .. }) { State::Sealed } else { holding.state.clone() };
    may_open(&check_state, &holding.seat, &machine.key, holding.pending.is_some())?;
    let cells = ctx.cells()?;
    same_wall(&holding.spec.wall, &cells.wall(&from).await?)?;
    let vault = ctx.home.vault()?;
    let keys = keys_for(&KeyRing::new(&store), eid)?;
    let facets = facets_of(&holding.spec, true)?;
    let access = cells.access(&from).await?;
    ctx.home.sweep_work();
    let work = ctx.home.run_dir()?;
    let sealed = seal_facets(&facets, &*access, &keys, &vault, &machine.signer, *eid, holding.seat.record.generation, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    let head = sealed?;
    // Order matters: every chunk and the manifest are in the vault before the
    // holding names the head, and the head is recorded before the state says
    // sealed. A kill between any two leaves a holding that still opens.
    abort_at("after_chunks");
    let manifest = load(&head, &keys, &vault)?;
    let chunks = chunks_of(&head, &manifest);
    abort_at("before_head_recorded");
    holding.record_head(serde_json::to_vec(&head)?, chunks.clone())?;
    abort_at("after_head_recorded");
    holding.set_state(State::Sealed)?;
    abort_at("after_state_sealed");
    cells.release(&from, eid)?;
    Ok(json!({ "ok": true, "eidolon": eid, "cell": from, "files": file_count(&manifest), "chunks": chunks.len(), "seconds": secs(started.elapsed()) }))
}

pub async fn open_cmd(ctx: &Ctx, eid: &EidolonId, cell: &str) -> Result<Value> {
    let started = Instant::now();
    let store = ctx.home.store()?;
    let machine = ctx.home.machine(&store)?;
    let mut holding = holding_of(ctx, eid)?;
    holding.may_open(&machine.key)?;
    let cells = ctx.cells()?;
    same_wall(&holding.spec.wall, &cells.wall(cell).await?)?;
    let vault = ctx.home.vault()?;
    let keys = keys_for(&KeyRing::new(&store), eid)?;
    let head = head_of(&holding)?;
    check_head(&holding.spec, &head)?;
    check_chunks(&head, &keys, &vault)?;
    let facets = facets_of(&holding.spec, false)?;
    let access = cells.access(cell).await?;
    // A browser needs a real cell set up for it. This comes before anything
    // is recorded or placed, so a refusal or a failed setup leaves nothing.
    let has_browser = browser::has_browser(&holding.spec);
    let provisioned = has_browser && browser::provision_if_needed(cells, cell).await?;
    // An open that was cut short elsewhere left files nothing else records.
    // They are cleared before this one starts, so one copy exists.
    let cleared = match holding.state.clone() {
        State::Opening { cell: earlier } if earlier != cell => {
            clear_unfinished(cells, &holding, &earlier).await?;
            Some(earlier)
        }
        _ => None,
    };
    // Recorded before the first file is placed: a kill from here on is
    // found, and finished or cleared, by the next command.
    holding.set_state(State::Opening { cell: cell.to_owned() })?;
    cells.hold(cell, eid)?;
    abort_at("after_state_opening");
    if has_browser {
        // A browser left running by the cut-short open would write over the profile.
        browser::stop(&*access).await?;
    }
    ctx.home.sweep_work();
    let work = ctx.home.run_dir()?;
    let placed = open_facets(&head, &facets, &*access, &keys, &vault, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    placed?;
    // The files are in the cell before the state says open, so open is never
    // recorded without them; a kill in between leaves it opening, and running
    // open again places them again.
    abort_at("before_state_open");
    if has_browser {
        browser::start(cells, cell).await?;
    }
    holding.set_state(State::Open { cell: cell.to_owned() })?;
    abort_at("after_state_open");
    Ok(json!({ "ok": true, "eidolon": eid, "cell": cell, "provisioned": provisioned, "cleared": cleared, "seconds": secs(started.elapsed()) }))
}

/// Removes what an open that never finished placed in `cell`, and lets the
/// cell go as holder. A cell that no longer exists has nothing to clear.
async fn clear_unfinished(cells: &dyn Cells, holding: &Holding, cell: &str) -> Result<()> {
    let eid = holding.spec.id;
    let access = match cells.access(cell).await {
        Ok(access) => access,
        Err(e) if matches!(e.code.as_str(), "unknown_cell" | "cell_missing") => {
            // Nothing there to hold or clear; a ledger that forgot the cell has no hold to drop.
            let _ = cells.release(cell, &eid);
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if browser::has_browser(&holding.spec) {
        browser::stop(&*access).await?;
    }
    let paths: Vec<String> = holding.spec.facets.iter().map(|f| f.path.clone()).collect();
    cells.clear(cell, &paths).await?;
    cells.release(cell, &eid)
}

pub fn verify_cmd(ctx: &Ctx, eid: &EidolonId) -> Result<Value> {
    let store = ctx.home.store()?;
    let holding = holding_of(ctx, eid)?;
    let vault = ctx.home.vault()?;
    let keys = keys_for(&KeyRing::new(&store), eid)?;
    let head = head_of(&holding)?;
    check_head(&holding.spec, &head)?;
    load(&head, &keys, &vault)?;
    let chunks = check_chunks(&head, &keys, &vault)?;
    Ok(json!({ "ok": true, "eidolon": eid, "chunks": chunks }))
}

// ---- move ---------------------------------------------------------------------

pub async fn move_cmd(ctx: &Ctx, eid: &EidolonId, argv: &[String]) -> Result<Value> {
    let store = ctx.home.store()?;
    let machine = ctx.home.machine(&store)?;
    let mut holding = holding_of(ctx, eid)?;
    if let State::Open { cell } = &holding.state {
        return Err(CliError::failed("second_seat", format!("That eidolon is open in {cell}, so it cannot be moved."), format!("Run `eidolon seal {eid}` first.")));
    }
    if let State::Opening { cell } = &holding.state {
        return Err(unfinished(eid, cell));
    }
    let vault = ctx.home.vault()?;
    let ring = KeyRing::new(&store);
    let (program, rest) = argv.split_first().ok_or_else(|| CliError::usage("`move` needs the receiving side's command.", "Put it after `--`."))?;
    // The child starts with an empty environment, so what it needs is named.
    let command = Command::new(program).args(rest).pass_env(&["PATH", "HOME", "EIDOLON_HOME", "SSH_AUTH_SOCK", "ADYTUM_HOME"]).deadline(Duration::from_secs(24 * 60 * 60));
    let mut duplex = SystemRunner::new().duplex(&command).await.map_err(|e| CliError::failed("receiver", e.to_string(), "Check the command after `--`."))?;
    let sent = send(&mut duplex, &mut holding, &vault, &ring, &machine.signer).await;
    let finished = duplex.finish().await;
    let stats = match sent {
        Ok(stats) => stats,
        Err(e) => {
            let mut error = CliError::from(e);
            // What the receiving command wrote on stderr says why, when it died.
            if let Ok(out) = &finished {
                let said = out.stderr_lossy();
                if !said.trim().is_empty() && error.code == "move" {
                    error.message = format!("{} The receiving side said: {}", error.message, said.trim());
                }
            }
            return Err(error);
        }
    };
    let receiver_status = finished.ok().and_then(|o| o.status);
    Ok(json!({
        "ok": true,
        "eidolon": eid,
        "generation": holding.seat.record.generation,
        "chunks_offered": stats.chunks_offered,
        "chunks_sent": stats.chunks_sent,
        "bytes_sent": stats.bytes_sent,
        "bytes_received": stats.bytes_received,
        "seconds": secs(stats.elapsed),
        "receiver_status": receiver_status,
    }))
}

pub async fn receive_cmd(ctx: &Ctx) -> Result<Value> {
    let store = ctx.home.store()?;
    let machine = ctx.home.machine(&store)?;
    let vault = ctx.home.vault()?;
    let ring = KeyRing::new(&store);
    // Standard output carries the move and nothing else.
    let mut stream = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    let got = receive(&mut stream, &ctx.home.holdings(), &vault, &ring, &machine.signer, &machine.key, &machine.seal_secret).await;
    let _ = stream.flush().await;
    let stats = got?;
    Ok(json!({
        "ok": true,
        "chunks_offered": stats.chunks_offered,
        "chunks_received": stats.chunks_sent,
        "bytes_sent": stats.bytes_sent,
        "bytes_received": stats.bytes_received,
        "seconds": secs(stats.elapsed),
    }))
}

pub fn settle_cmd(ctx: &Ctx, eid: &EidolonId, how: &Settle, stdin: &mut dyn FnMut() -> std::io::Result<Vec<u8>>) -> Result<Value> {
    let store = ctx.home.store()?;
    let mut holding = holding_of(ctx, eid)?;
    match how {
        Settle::Abandon => {
            holding.abandon_pending()?;
            Ok(json!({ "ok": true, "eidolon": eid, "settled": "abandoned" }))
        }
        Settle::Confirm(from) => {
            let bytes = if from == "-" {
                stdin().map_err(|e| CliError::io("read standard input", &e))?
            } else {
                std::fs::read(from).map_err(|e| CliError::io(&format!("read {from}"), &e))?
            };
            let taken: SignedSeat = serde_json::from_slice(&bytes)?;
            holding.confirm_moved(&KeyRing::new(&store), &taken)?;
            Ok(json!({ "ok": true, "eidolon": eid, "settled": "confirmed", "generation": taken.record.generation }))
        }
    }
}

// ---- looking ------------------------------------------------------------------

pub fn facets_cmd(ctx: &Ctx, eid: &EidolonId) -> Result<Value> {
    let holding = holding_of(ctx, eid)?;
    Ok(json!({ "ok": true, "eidolon": eid, "facets": holding.spec.facets }))
}

pub fn ls_cmd(ctx: &Ctx) -> Result<Value> {
    let store = ctx.home.store()?;
    let me = ctx.home.machine(&store).ok().map(|m| m.key);
    let mut rows = Vec::new();
    let root = ctx.home.holdings();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({ "ok": true, "eidolons": rows })),
        Err(e) => return Err(CliError::io("read the holdings", &e)),
    };
    let mut ids: Vec<EidolonId> = entries.filter_map(std::result::Result::ok).filter_map(|e| e.file_name().to_str().and_then(|n| n.parse().ok())).collect();
    ids.sort();
    for id in ids {
        if !Holding::exists(&root, &id) {
            continue;
        }
        let h = Holding::load(&root, &id)?;
        rows.push(json!({
            "eidolon": id,
            "wall": h.spec.wall,
            "state": h.state,
            "generation": h.seat.record.generation,
            "seat_here": me.is_some_and(|m| h.seat.record.holder == m),
            "pending_move": h.pending.is_some(),
            "facets": h.spec.facets.len(),
        }));
    }
    Ok(json!({ "ok": true, "eidolons": rows }))
}

// ---- the browser: the window and the tools --------------------------------------

/// Where an open eidolon with a browser lives: its cell, the cell's driver
/// and the marker that says whether the sign-in window is open.
struct Browsing {
    cell: String,
    driver: std::sync::Arc<dyn adytum_driver::Driver>,
    name: adytum_model::CellName,
    guard: ViewGuard,
}

async fn browsing(ctx: &Ctx, eid: &EidolonId) -> Result<Browsing> {
    let holding = holding_of(ctx, eid)?;
    let State::Open { cell } = &holding.state else {
        return Err(CliError::failed("not_open", "That eidolon is not open in a cell.", format!("Run `eidolon open {eid} --cell <cell>` first.")));
    };
    if !browser::has_browser(&holding.spec) {
        return Err(CliError::failed("no_browser", "That eidolon has no browser facet.", "Make one with `eidolon new ... --browser <name>`."));
    }
    // The marker lives in the holding's folder, beside the eidolon's state.
    // It is read before the cell is touched: while a window is open, nothing
    // else drives the browser, and nothing starts the cell for it.
    let guard = ViewGuard::new(ctx.home.holdings().join(eid.to_string()));
    guard.refuse_if_open()?;
    let cells = ctx.cells()?;
    same_wall(&holding.spec.wall, &cells.wall(cell).await?)?;
    let (driver, name) = cells.driver(cell).await?;
    Ok(Browsing { cell: cell.clone(), driver, name, guard })
}

/// Opens the sign-in window on the browser of an open eidolon. The caller
/// shows the address and the secret, waits for the person, and closes it.
pub async fn view_open(ctx: &Ctx, eid: &EidolonId) -> Result<(View, Value)> {
    let b = browsing(ctx, eid).await?;
    let mut view = View::open_with(b.driver, b.name, eidolon_browser::units::Ports::default(), OpenOptions { guard: Some(b.guard), ..OpenOptions::default() }).await?;
    let secret = view.secret();
    let info = json!({ "ok": true, "event": "open", "eidolon": eid, "cell": b.cell, "address": view.address().to_string(), "url": view.url(), "secret": secret });
    Ok((view, info))
}

/// What `mcp` is serving, once the browser is reached.
pub struct McpSetup {
    pub tools: eidolon_mcp::BrowserTools,
    pub cell: String,
    pub files: std::path::PathBuf,
}

/// The tools for the browser of an open eidolon. Refused while the sign-in
/// window is open: automation is off then.
pub async fn mcp_setup(ctx: &Ctx, eid: &EidolonId) -> Result<McpSetup> {
    let b = browsing(ctx, eid).await?;
    let files = ctx.home.files_dir(eid)?;
    let browser = std::sync::Arc::new(eidolon_browser::Browser::supervise(
        b.driver.clone(),
        b.name.clone(),
        eidolon_browser::units::DEFAULT_AUTOMATION_PORT,
        eidolon_browser::ConnectOptions::default(),
    ));
    let tools = eidolon_mcp::BrowserTools::new(browser, b.driver, b.name, files.clone());
    Ok(McpSetup { tools, cell: b.cell, files })
}
