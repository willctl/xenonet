use std::io::Write;
use std::path::{Path, PathBuf};

use adytum_driver::{cell_name, Driver, DriverError, ExecOpts};
use adytum_ledger::{create_cell, reconcile, remove_cell, Ledger, LedgerError, OpKind, Outcome, Report};
use adytum_model::{CellName, CellSpec, CellState, DriverKind, Port, Resources, Wall};
use serde_json::{json, Value};

use crate::args::{Command, CreateArgs, Usage};
use crate::{Context, Done, Failure};

const DEFAULT_WALL: &str = "wall_default";

/// The ledger, open, and what reconciling it did.
struct Session {
    ledger: Ledger,
    report: Report,
}

impl Session {
    /// Opens the ledger and reconciles it with the driver before anything
    /// else happens. Says on `err` what it did, if it did anything.
    async fn open(ctx: &Context, err: &mut dyn Write) -> Result<Session, Failure> {
        let ledger = Ledger::open(&ctx.home)?;
        let report = reconcile(&ledger, &*ctx.driver).await?;
        for (what, names) in [("removed the half-made or vanished cell", &report.removed), ("finished removing", &report.finished)] {
            for name in names {
                // A closed pipe is the reader's choice.
                let _ = writeln!(err, "adytum: reconciled: {what} {name}.");
            }
        }
        Ok(Session { ledger, report })
    }

    fn reconciled(&self) -> Value {
        json!({ "removed": self.report.removed, "finished": self.report.finished, "unrecorded": self.report.adopted_none })
    }

    fn done(&self, mut json: Value, text: String) -> Done {
        if let Value::Object(map) = &mut json {
            map.insert("reconciled".to_owned(), self.reconciled());
        }
        Done::new(json, text)
    }

    /// The recorded cell's spec, or `NotRecorded`: Adytum touches only what it wrote down.
    fn recorded(&self, name: &CellName) -> Result<CellSpec, Failure> {
        self.ledger.cells()?.into_iter().find(|c| &c.spec.name == name).map(|c| c.spec).ok_or_else(|| Failure::from(LedgerError::NotRecorded(name.clone())))
    }
}

pub(crate) async fn execute(command: Command, ctx: &Context, out: &mut dyn Write, err: &mut dyn Write, json: bool) -> Result<Done, Failure> {
    // Names and options are checked before the ledger is opened, so a
    // refused line leaves no trace on disk.
    match command {
        Command::Help => Ok(Done::new(json!({}), String::new())),
        Command::Doctor => doctor().await,
        Command::Create(args) => {
            let spec = spec_from(&args, cell_name(&args.name)?)?;
            create(ctx, err, spec).await
        }
        Command::Start(name) => {
            let name = cell_name(&name)?;
            let session = Session::open(ctx, err).await?;
            let op = session.ledger.intend(OpKind::Start, &name)?;
            let result = ctx.driver.start(&name).await;
            settle(&session.ledger, op, &result)?;
            result?;
            Ok(session.done(json!({ "name": name, "state": CellState::Running }), format!("started {name}")))
        }
        Command::Stop(name) => {
            let name = cell_name(&name)?;
            let session = Session::open(ctx, err).await?;
            let op = session.ledger.intend(OpKind::Stop, &name)?;
            let result = ctx.driver.stop(&name).await;
            settle(&session.ledger, op, &result)?;
            result?;
            Ok(session.done(json!({ "name": name, "state": CellState::Stopped }), format!("stopped {name}")))
        }
        Command::Rm(name) => {
            let name = cell_name(&name)?;
            let session = Session::open(ctx, err).await?;
            remove_cell(&session.ledger, &*ctx.driver, &name).await?;
            Ok(session.done(json!({ "name": name, "removed": true }), format!("removed {name}")))
        }
        Command::Clone { from, to, wall, ports } => {
            let (from, to) = (cell_name(&from)?, cell_name(&to)?);
            let wall = wall.map(|w| w.parse::<Wall>().map_err(|e| usage(format!("{e}."), "Write a wall like wall_work."))).transpose()?;
            clone(ctx, err, &from, to, wall, &ports).await
        }
        Command::Hold { name, eidolon } => {
            let name = cell_name(&name)?;
            let session = Session::open(ctx, err).await?;
            session.ledger.hold(&name, &eidolon)?;
            Ok(session.done(json!({ "name": name, "eidolon": eidolon, "held": true }), format!("{name} holds {eidolon}")))
        }
        Command::Release { name, eidolon } => {
            let name = cell_name(&name)?;
            let session = Session::open(ctx, err).await?;
            session.ledger.release(&name, &eidolon)?;
            Ok(session.done(json!({ "name": name, "eidolon": eidolon, "held": false }), format!("{name} no longer holds {eidolon}")))
        }
        Command::Exec { name, argv } => exec(ctx, &name, &argv, out, err, json).await,
        Command::Cp { src, dst } => copy(ctx, err, &src, &dst).await,
        Command::Ls => list(ctx, err).await,
    }
}

/// Closes an operation with how the driver did. A failed settle is reported
/// only when the driver itself worked: the driver's error matters more.
fn settle<T>(ledger: &Ledger, op: adytum_ledger::OpId, result: &Result<T, DriverError>) -> Result<(), Failure> {
    let outcome = match result {
        Ok(_) => Outcome::Done,
        Err(e) => Outcome::Failed(e.to_string()),
    };
    match ledger.settle(op, outcome) {
        Err(e) if result.is_ok() => Err(e.into()),
        _ => Ok(()),
    }
}

async fn doctor() -> Result<Done, Failure> {
    let checks = adytum_doctor::checks().await;
    let failed = adytum_doctor::has_failure(&checks);
    Ok(Done { json: json!({ "ok": !failed, "checks": checks }), text: adytum_doctor::render_human(&checks).trim_end().to_owned(), exit: i32::from(failed) })
}

fn usage(message: impl Into<String>, fix: impl Into<String>) -> Failure {
    Failure::from(Usage { message: message.into(), fix: fix.into() })
}

fn spec_from(args: &CreateArgs, name: CellName) -> Result<CellSpec, Failure> {
    let image_name = args.image.as_deref().unwrap_or(adytum_lima::DEFAULT_IMAGE);
    let image = adytum_lima::image(image_name)
        .ok_or_else(|| usage(format!("There is no image named {image_name:?}."), format!("Use one of: {}.", adytum_lima::image_names().join(", "))))?;
    let wall: Wall = args.wall.as_deref().unwrap_or(DEFAULT_WALL).parse().map_err(|e| usage(format!("{e}."), "Write a wall like wall_work."))?;
    let defaults = Resources::default();
    let resources = Resources { cpus: args.cpus.unwrap_or(defaults.cpus), memory_mib: args.memory_mib.unwrap_or(defaults.memory_mib), disk_gib: defaults.disk_gib };
    if resources.cpus == 0 || resources.memory_mib == 0 {
        return Err(usage("A cell needs at least one CPU and some memory.", "Give --cpus and --memory a number above 0."));
    }
    Ok(CellSpec {
        name,
        driver: DriverKind::Lima,
        image,
        wall,
        resources,
        ports: loopback_ports(&args.ports),
        automation_port: args.automation_port,
        mounts: Vec::new(),
        sealed: args.sealed,
        network: None,
        clone_from: None,
        secrets: Vec::new(),
    })
}

fn loopback_ports(ports: &[(u16, u16)]) -> Vec<Port> {
    let loopback = std::net::IpAddr::from([127, 0, 0, 1]);
    ports.iter().map(|&(host, guest)| Port { guest, host, bind: loopback }).collect()
}

async fn create(ctx: &Context, err: &mut dyn Write, spec: CellSpec) -> Result<Done, Failure> {
    let session = Session::open(ctx, err).await?;
    let id = create_cell(&session.ledger, &*ctx.driver, &spec).await?;
    let sealed = if spec.sealed { ", sealed" } else { "" };
    Ok(session.done(
        json!({ "name": spec.name, "id": id.to_string(), "state": CellState::Stopped, "wall": spec.wall, "sealed": spec.sealed }),
        format!("created {} ({}{sealed}); start it with `adytum start {}`", spec.name, spec.wall, spec.name),
    ))
}

/// Makes `to` as a copy of the stopped cell `from`. The new cell's spec is
/// the source's, with the new name, the wall asked for (the source's by
/// default) and only the ports asked for, since a published host port is
/// the source's alone. The rules run against the ledger's world, so a clone
/// across walls, or of a cell that holds an open eidolon, is refused before
/// anything is written. Then the same order as a create: intent, driver,
/// settle; a failed clone is removed, and a crash is cleaned up by the next
/// reconcile.
async fn clone(ctx: &Context, err: &mut dyn Write, from: &CellName, to: CellName, wall: Option<Wall>, ports: &[(u16, u16)]) -> Result<Done, Failure> {
    let session = Session::open(ctx, err).await?;
    let source = session.recorded(from)?;
    let spec = CellSpec { name: to, wall: wall.unwrap_or_else(|| source.wall.clone()), ports: loopback_ports(ports), clone_from: Some(from.clone()), ..source };
    adytum_policy::check_create(&spec, &session.ledger.world()?).map_err(DriverError::Refused)?;
    if ctx.driver.list().await?.iter().any(|l| l.name == spec.name) {
        return Err(Failure::from(DriverError::Refused(adytum_model::Refusal::new(
            adytum_model::RefusalCode::NameTaken,
            format!("The driver already has a cell named {} that Adytum did not make.", spec.name),
            "Pick another name; Adytum does not touch cells it did not record.",
        ))));
    }
    let op = session.ledger.intend_create(&spec)?;
    match Driver::clone(&*ctx.driver, from, &spec).await {
        Ok(()) => session.ledger.settle(op, Outcome::Done)?,
        Err(e) => {
            // Take away whatever half-made cell there is; if that fails the
            // operation stays open and the next reconcile tries again.
            if matches!(ctx.driver.remove(&spec.name).await, Ok(()) | Err(DriverError::NotFound { .. })) {
                session.ledger.settle(op, Outcome::Failed(e.to_string()))?;
            }
            return Err(e.into());
        }
    }
    Ok(session.done(
        json!({ "name": spec.name, "state": CellState::Stopped, "wall": spec.wall, "sealed": spec.sealed, "cloned_from": from }),
        format!("cloned {from} to {} ({}); start it with `adytum start {}`", spec.name, spec.wall, spec.name),
    ))
}

async fn exec(ctx: &Context, name: &str, argv: &[String], out: &mut dyn Write, err: &mut dyn Write, json: bool) -> Result<Done, Failure> {
    let name = cell_name(name)?;
    let session = Session::open(ctx, err).await?;
    session.recorded(&name)?;
    let output = ctx.driver.exec(&name, argv, ExecOpts::default()).await?;
    let exit = i32::from(!output.success());
    if json {
        let status = output.status;
        let mut done = session.done(json!({ "name": name, "status": status, "stdout": output.stdout_lossy(), "stderr": output.stderr_lossy() }), String::new());
        done.exit = exit;
        return Ok(done);
    }
    // A closed pipe is the reader's choice.
    let _ = out.write_all(&output.stdout);
    let _ = err.write_all(&output.stderr);
    if exit != 0 {
        let how = output.status.map_or_else(|| "was stopped".to_owned(), |s| format!("exited with status {s}"));
        let _ = writeln!(err, "adytum: the command {how}.");
    }
    Ok(Done { json: json!({}), text: String::new(), exit })
}

/// `<name>:<path>` is a path inside a cell; anything else is on the host.
fn guest_side(side: &str) -> Option<(&str, &str)> {
    let (name, path) = side.split_once(':')?;
    (!name.is_empty() && !name.contains('/')).then_some((name, path))
}

fn absolute(path: &str) -> Result<PathBuf, Failure> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|source| Failure::from(DriverError::Io { what: "read the current folder", source }))?;
    Ok(cwd.join(path))
}

async fn copy(ctx: &Context, err: &mut dyn Write, src: &str, dst: &str) -> Result<Done, Failure> {
    let (name, to_cell) = match (guest_side(src), guest_side(dst)) {
        (None, Some((name, _))) => (cell_name(name)?, true),
        (Some((name, _)), None) => (cell_name(name)?, false),
        (Some(_), Some(_)) => return Err(usage("Both sides are paths inside cells.", "Copy through the host in two steps.")),
        (None, None) => return Err(usage("Neither side is a path inside a cell.", "Write one side as <name>:<absolute path>; start a host path with ./ if it holds a colon.")),
    };
    let session = Session::open(ctx, err).await?;
    session.recorded(&name)?;
    if to_cell {
        let path = guest_side(dst).map_or("", |(_, p)| p);
        ctx.driver.copy_in(&name, &absolute(src)?, path).await?;
    } else {
        let path = guest_side(src).map_or("", |(_, p)| p);
        ctx.driver.copy_out(&name, path, &absolute(dst)?).await?;
    }
    Ok(session.done(json!({ "from": src, "to": dst }), format!("copied {src} to {dst}")))
}

async fn list(ctx: &Context, err: &mut dyn Write) -> Result<Done, Failure> {
    let session = Session::open(ctx, err).await?;
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for cell in session.ledger.cells()? {
        let name = &cell.spec.name;
        rows.push(json!({ "name": name, "state": cell.state, "wall": cell.spec.wall }));
        let state = serde_json::to_value(cell.state).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        lines.push(format!("{name}  {state}  {}", cell.spec.wall));
    }
    // Cells the driver has that Adytum never recorded: shown, never touched.
    for name in &session.report.adopted_none {
        lines.push(format!("{name}  (not recorded by adytum; left alone)"));
    }
    let text = if lines.is_empty() { "no cells".to_owned() } else { lines.join("\n") };
    Ok(session.done(json!({ "cells": rows }), text))
}
