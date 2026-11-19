# Adytum

Adytum makes, runs and removes cells on one machine. A cell is a VM, or a container, with one purpose, behind a wall that Adytum keeps apart from every other wall.

It sits beside XenoNet in this repository. XenoNet decides who may do what across machines, and Adytum only acts for the person on the machine it runs on.

[Preview the draft marks](brand/drafts.html) for Adytum and Eidolon.

## Build

```text
npm run cargo -- build -p adytum-cli -p eidolon-cli
```

The binary is `target/debug/adytum`. Cells need [Lima](https://lima-vm.io) 2.1.2 on the machine; `adytum doctor` says what is missing.

## Commands

Every command takes `--json`, which prints one JSON object on standard output. An error is `{"error": {"code", "message", "fix"}}`. Exit 0 is done, 1 is a refusal or a failure, 2 is a mistake in how it was typed.

| Command | What it does |
|---|---|
| `adytum create <name> [--sealed] [--wall wall_x] [--cpus N] [--memory MiB] [--image ubuntu-24.04] [--port host:guest]... [--automation-port N]` | Records and makes a stopped cell |
| `adytum start <name>`, `adytum stop <name>`, `adytum rm <name>` | Runs, stops and removes a cell |
| `adytum clone <from> <to> [--wall wall_x] [--port host:guest]...` | Copies a stopped cell |
| `adytum exec <name> -- <command> [args...]` | Runs a program in a cell, as an argument list |
| `adytum cp <source> <destination>` | Copies a file; one side is `<name>:<absolute path>` |
| `adytum hold <name> <eidolon>`, `adytum release <name> <eidolon>` | Records that a cell holds an open eidolon, or no longer does |
| `adytum ls` | Lists the cells, their state and their wall |
| `adytum doctor` | Says what is wrong with the setup, each problem with its fix |

Cell names are `adytum-` followed by lowercase letters, digits and dashes. A wall is written `wall_` and a word, such as `wall_work`; without `--wall`, a cell gets `wall_default`.

A sealed cell with a published port, then a command in it:

```text
adytum create adytum-web --sealed --wall wall_work --port 18080:80
adytum start adytum-web
adytum exec adytum-web -- uname -a
```

## The rules

Each rule starts as a test that proves the refusal.

- A sealed cell has no mounts from the host.
- A port is published on 127.0.0.1 only. A cell's automation port is never published at all.
- Adytum touches only names that start with `adytum-`. Every driver refuses any other name.
- Adytum runs Lima in a folder of its own, `~/.adytum/lima` (or under `ADYTUM_HOME`), so the VMs you keep in Lima's default folder are not listed and cannot be reached.
- Two cells with different walls never share a network, a mount source, a clone source or a secret.
- A cell that holds an open eidolon is not cloned.
- A command is a program and an argument list, never a shell string. The child gets an empty environment plus the variables named for it, and the whole process group is stopped at the deadline.
- The ledger (`~/.adytum/adytum.sqlite`) records a cell before it is created or removed and settles the record after. Every command compares the ledger with what Lima reports first and finishes or removes the difference, so a crash cannot leave a cell behind. A cell the ledger never recorded is listed and never touched.

## Limits

- Lima VMs are the strong wall. A container (`adytum-oci`, for podman or docker) shares the host's kernel, so a flaw in the kernel reaches the host. The `adytum` command makes Lima cells only for now.
- There is no Windows or WSL driver yet.
- A VM needs 20 GiB free where `~/.adytum` lives; `adytum doctor` checks it.

## Where the code is

`adytum/crates/`, members of the root workspace.

| Crate | Job |
|---|---|
| `adytum-model` | Typed ids, names, the cell spec, image pins, refusals |
| `adytum-policy` | Every isolation rule as a pure function |
| `adytum-exec` | Running a program as an argument list, with a deadline and a cancel |
| `adytum-driver` | The trait a driver implements, and its errors |
| `adytum-lima` | Lima cells, from a pinned image |
| `adytum-oci` | Podman and docker cells |
| `adytum-ledger` | The record before and after each create and remove, and reconcile |
| `adytum-doctor` | What is wrong, with the fix |
| `adytum-cli` | The `adytum` binary |
| `adytum-testkit` | A fake `limactl` and `podman` that replay golden files |

## Tests

```text
npm run cargo -- test -p adytum-cli
npm run cargo -- test -p adytum-policy
```

The live test makes real VMs, names them `adytum-test-live-*` in a Lima folder of its own, and removes them and the folder at the end, also when a step fails.

```text
ADYTUM_LIVE=1 npm run cargo -- test -p adytum-cli --test live -- --nocapture
```

## More

- [The guide page](../docs/guide/adytum.md) for using it with XenoNet.
- [Release notes](RELEASES.md).
- [Eidolon](../eidolon/README.md), which keeps an entity's state and opens it in a cell.
- [The design](../docs/design/cells.md).
