use std::io::Write;
use std::path::{Path, PathBuf};

use adytum_driver::{cell_name, ExecOpts};
use adytum_model::{CellName, CellSpec, CellState, DriverKind, Port, Resources, Wall};
use adytum_policy::World;
use serde_json::json;

use crate::args::{Command, CreateArgs, Usage};
use crate::records::{self, Record};
use crate::{Context, Done, Failure};

const DEFAULT_WALL: &str = "wall_default";

pub(crate) async fn execute(command: Command, ctx: &Context, out: &mut dyn Write, err: &mut dyn Write, json: bool) -> Result<Done, Failure> {
    match command {
        Command::Help => Ok(done(json!({}), String::new())),
        Command::Create(args) => create(ctx, args).await,
        Command::Start(name) => {
            let name = cell_name(&name)?;
            ctx.driver.start(&name).await?;
            Ok(done(json!({ "name": name, "state": CellState::Running }), format!("started {name}")))
        }
        Command::Stop(name) => {
            let name = cell_name(&name)?;
            ctx.driver.stop(&name).await?;
            Ok(done(json!({ "name": name, "state": CellState::Stopped }), format!("stopped {name}")))
        }
        Command::Rm(name) => {
            let name = cell_name(&name)?;
            ctx.driver.remove(&name).await?;
            // The cell is gone; a record that cannot be deleted is only clutter.
            let _ = records::remove(&ctx.home, &name);
            Ok(done(json!({ "name": name, "removed": true }), format!("removed {name}")))
        }
        Command::Exec { name, argv } => exec(ctx, &name, &argv, out, err, json).await,
        Command::Cp { src, dst } => copy(ctx, &src, &dst).await,
        Command::Ls => list(ctx).await,
    }
}

fn done(json: serde_json::Value, text: String) -> Done {
    Done { json, text, exit: 0 }
}

fn usage(message: impl Into<String>, fix: impl Into<String>) -> Failure {
    Failure::from(Usage { message: message.into(), fix: fix.into() })
}

async fn world(ctx: &Context) -> Result<World, Failure> {
    let cells = ctx.driver.list().await?;
    Ok(World { cells: cells.iter().map(|c| records::read(&ctx.home, &c.name).map_or_else(|| records::unrecorded(&c.name), |r| r.facts())).collect() })
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
    let loopback = std::net::IpAddr::from([127, 0, 0, 1]);
    Ok(CellSpec {
        name,
        driver: DriverKind::Lima,
        image,
        wall,
        resources,
        ports: args.ports.iter().map(|&(host, guest)| Port { guest, host, bind: loopback }).collect(),
        automation_port: args.automation_port,
        mounts: Vec::new(),
        sealed: args.sealed,
        network: None,
        clone_from: None,
        secrets: Vec::new(),
    })
}

async fn create(ctx: &Context, args: CreateArgs) -> Result<Done, Failure> {
    let name = cell_name(&args.name)?;
    let spec = spec_from(&args, name)?;
    adytum_policy::check_create(&spec, &world(ctx).await?).map_err(adytum_driver::DriverError::Refused)?;
    ctx.driver.create(&spec).await?;
    let record = Record {
        name: spec.name.clone(),
        wall: spec.wall.clone(),
        sealed: spec.sealed,
        network: spec.network.clone(),
        mount_sources: spec.mounts.iter().map(|m| m.source.clone()).collect(),
        secrets: spec.secrets.iter().map(|s| s.name.clone()).collect(),
    };
    records::write(&ctx.home, &record).map_err(|source| Failure::from(adytum_driver::DriverError::Io { what: "write the cell's record", source }))?;
    let sealed = if spec.sealed { ", sealed" } else { "" };
    Ok(done(
        json!({ "name": spec.name, "state": CellState::Stopped, "wall": spec.wall, "sealed": spec.sealed }),
        format!("created {} ({}{sealed}); start it with `adytum start {}`", spec.name, spec.wall, spec.name),
    ))
}

async fn exec(ctx: &Context, name: &str, argv: &[String], out: &mut dyn Write, err: &mut dyn Write, json: bool) -> Result<Done, Failure> {
    let name = cell_name(name)?;
    let output = ctx.driver.exec(&name, argv, ExecOpts::default()).await?;
    let exit = i32::from(!output.success());
    if json {
        let status = output.status;
        return Ok(Done { json: json!({ "name": name, "status": status, "stdout": output.stdout_lossy(), "stderr": output.stderr_lossy() }), text: String::new(), exit });
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
    let cwd = std::env::current_dir().map_err(|source| Failure::from(adytum_driver::DriverError::Io { what: "read the current folder", source }))?;
    Ok(cwd.join(path))
}

async fn copy(ctx: &Context, src: &str, dst: &str) -> Result<Done, Failure> {
    match (guest_side(src), guest_side(dst)) {
        (None, Some((name, path))) => {
            let name = cell_name(name)?;
            ctx.driver.copy_in(&name, &absolute(src)?, path).await?;
        }
        (Some((name, path)), None) => {
            let name = cell_name(name)?;
            ctx.driver.copy_out(&name, path, &absolute(dst)?).await?;
        }
        (Some(_), Some(_)) => return Err(usage("Both sides are paths inside cells.", "Copy through the host in two steps.")),
        (None, None) => return Err(usage("Neither side is a path inside a cell.", "Write one side as <name>:<absolute path>; start a host path with ./ if it holds a colon.")),
    }
    Ok(done(json!({ "from": src, "to": dst }), format!("copied {src} to {dst}")))
}

async fn list(ctx: &Context) -> Result<Done, Failure> {
    let cells = ctx.driver.list().await?;
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for cell in &cells {
        let wall = records::read(&ctx.home, &cell.name).map(|r| r.wall);
        rows.push(json!({ "name": cell.name, "state": cell.state, "wall": wall }));
        let state = serde_json::to_value(cell.state).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        lines.push(format!("{}  {state}  {}", cell.name, wall.map_or_else(String::new, |w| w.to_string())).trim_end().to_owned());
    }
    let text = if lines.is_empty() { "no cells".to_owned() } else { lines.join("\n") };
    Ok(done(json!({ "cells": rows }), text))
}
