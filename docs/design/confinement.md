# Confinement

XenoNet confines work through [Adytum](https://github.com/willctl/adytum), a Rust library in its own repository (one crate, `adytum`) that makes sealed cells (Lima VMs, OpenShell sandboxes) and enforces walls between them. XenoNet decides who may run what. Adytum makes the cell and keeps the wall. Adytum depends on nothing of XenoNet's.

Related: [authorization](authorization.md), [rust-core](rust-core.md) (the layering), [integrations](../guide/integrations.md) (OpenShell jobs).

## What uses Adytum

| Where | What it takes from Adytum |
|---|---|
| `adapters/acp` (agent starts over ACP) | `adytum::exec`'s `Spawned`. The agent starts with an empty environment plus the variables the start names, in its own process group. Stopping it kills the group. On Windows a job object kills the tree. |
| `xenonet-service`'s one-shot runs (`compose/runtime/runs.rs`) | `Spawned` too, with the turn's deadline. The pid is recorded at spawn, output is logged live, and a stop kills the group. |
| `xenonet-service`'s OpenShell jobs (`compose/integrations/openshell.rs`) | `adytum::openshell` (the `openshell` feature). It provides each command's argument list, the native or WSL profile, the policy file's hash, the `policy_denied` check and sandbox names. XenoNet keeps the job records, the confirm-before-upload step, retrieval and reviews. |
| `xenonet doctor`, cells group | `adytum::doctor` (the `doctor` feature): it reads the machine and its findings become the check's verdict, where the `adytum` command is installed. |
| `xenonet-engine::confine`, `xenonet-service::cells` | The `cell` confinement level, below: `Wall` in the engine; `adytum::Cells` and `adytum::lima` (the `lima` feature) in the runner, with `adytum::testing`'s fake driver in its tests. |

`scripts/rust-layers.mjs` holds the rule:
- The workspace lists `adytum` with no features: its types, wall rules and process runner, a library any crate may use.
- Only `xenonet-service`, `xenonet-cli` and `xenonet-doctor` turn on its features (`lima`, `openshell`, `doctor`), and `testing` only in dev-dependencies.
- Eidolon's crates are used only by `xenonet-service` and `xenonet-cli`.

The engine reaches a cell through a trait, as it reaches platforms.

## The `cell` level

A start can be confined at one of three levels:
- `off`;
- `vendor`, the harness's own sandbox (the daily-use plan's slice 4);
- `cell`, a sealed Adytum cell behind the start's wall.

The engine's rules are in `crates/xenonet-engine/src/confine.rs`. Each refusal has its test.
- A missing level is refused, never defaulted. A `cell` start needs a wall, and only a `cell` start may name one.
- The wall is the starting identity's own: `wall_`, a slug of the identity id, `_`, then the first 16 hex digits of the id's SHA-256.
- The cell gets `PATH` and `HOME` (guest values) plus `LANG`, `LC_ALL`, `LC_CTYPE`, `TERM`, `TZ` and `NO_COLOR`, and nothing else. A value or argument holding the host's home path is refused.
- The project folder's path is resolved before any check:
  - Refused: a symlink at the top, the home folder or any folder holding it (in any letter case on macOS and Windows), dot folders and `Library` under home, and Adytum's and XenoNet's own folders.
  - The plan keeps the folder's device and inode, and a folder swapped before the run is refused.
- An app runs in a cell only if the cell's image has a Linux build of it. Otherwise it is refused in words.

The runner is `crates/xenonet-service/src/cells.rs`. It works in these steps:
1. Copy the project into a staged copy that leaves out `.env`, `.env.*`, `.git/config` and every symlink or special file.
2. Make a sealed cell: no mounts, no ports, no secrets.
3. Copy the staged project in, and run the program under `env -i` in the copy. The cell's user makes the program's home and writes the copy, so it owns both.
4. Bring results back only into a fresh folder of its own. A result holding a link or a special file is refused and removed.
5. Remove the cell, also when the run fails or panics. A cell that can't be removed is an error.

Every create, start and remove goes through `adytum::Cells`, which records it in Adytum's ledger first. `CellRunner::reconcile_now` settles the ledger.

Inside a Lima cell, Adytum's boot-time provisioning keeps the wall:
- Commands run as the `cell` user, which has no sudo.
- An nftables guard on output and forward drops traffic to the host (`192.168.5.2`, `host.lima.internal`, the gateway) and to private ranges, the host's LAN address included. DNS to Lima's resolver still works.
- The internet stays open.

The live test `crates/xenonet-service/tests/cells_live.rs` (`ADYTUM_LIVE=1`) checks each of these against a real VM.

## Not wired yet

Nothing sets the level yet. Slice 4 of the daily-use work adds it to the start request and the signed act. When it does:
- the identity comes from the verified act, never the request;
- the level and the wall are on the signed act;
- `keep` is the owner's choice, and a kept cell is bound to its wall;
- the arguments come from the adapter.

Today every cell shares one Adytum home and one Lima SSH key, so a wall separates cells by name, ledger and network, not by key. Separate homes or keys per wall come before guests use the `cell` level.

Lima named networks join cells behind one wall over Lima's `user-v2` switch. UDP crosses within a wall's network, and nothing crosses between walls. Multicast doesn't cross, so the lab's mDNS checks still need `socket_vmnet`.
