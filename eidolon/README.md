# Eidolon

Eidolon keeps an entity's state as one sealed thing, an eidolon, which opens in an Adytum cell and moves between machines. An eidolon is a set of facets, a folder or a browser profile, and exactly one machine holds its seat at a time.

It sits beside XenoNet in this repository. XenoNet stays the only way across machines, and Eidolon opens no network listener beyond 127.0.0.1.

[Preview the draft marks](../adytum/brand/drafts.html) for Adytum and Eidolon.

## Build

```text
npm run cargo -- build -p adytum-cli -p eidolon-cli
```

The binary is `target/debug/eidolon`. It needs Adytum's cells, so run `adytum doctor` first ([Adytum](../adytum/README.md)).

## Commands

Every command takes `--json`. Exit 0 is done, 1 is a refusal or a failure, 2 is a mistake in how it was typed. Errors read `eidolon: <what happened> <what to do>` on standard error.

| Command | What it does |
|---|---|
| `eidolon machine init --name <name> [--key-store file]` | Makes this machine's keys and name |
| `eidolon machine show` | Prints this machine's holder entry, to give to other machines |
| `eidolon new --wall <wall_x> --holder <holder.json> ... [--folder <name>=<guest path>] [--browser <name>] [--leave-out <glob>]...` | Makes a sealed, empty eidolon; each `--leave-out` belongs to the facet before it |
| `eidolon seal <eid> [--cell <cell>]` | Takes the files of the facets from the cell into the vault; the first seal names the cell |
| `eidolon open <eid> --cell <cell>` | Lays the eidolon out in a cell behind its wall |
| `eidolon move <eid> -- <command>` | Sends the eidolon to the machine that the command reaches |
| `eidolon receive` | The receiving side of a move, reading standard input |
| `eidolon move-settle <eid> --abandon` or `--confirm <seat.json>` | Settles a move that was cut short |
| `eidolon verify <eid>` | Checks every chunk of the head against the vault |
| `eidolon facets <eid>`, `eidolon ls` | Lists an eidolon's facets, or the eidolons here |
| `eidolon view <eid>` | Opens the sign-in window on a browser facet |
| `eidolon mcp <eid> [--http 127.0.0.1:<port>]` | Serves the browser's tools to an agent |

A folder facet from a first seal to a move:

```text
eidolon machine init --name desk
eidolon machine show > desk.holder.json
eidolon new --wall wall_work --holder desk.holder.json --folder notes=/tmp/notes --leave-out cache
eidolon seal <eid> --cell adytum-a
eidolon move <eid> -- ssh tower eidolon receive
```

## The rules

Each rule starts as a test that proves the refusal.

- One seat. The seat is a signed record with a generation number. A move writes the next generation, naming the receiver, signed by the sender, and the sender then deletes its eidolon key. A receiver refuses a record that is not the next generation, not signed by the current seat, or names a machine off the holder list.
- The holder list. An eidolon can only be received by, or opened on, a machine on its list, named by its Ed25519 key. The head must be signed by a machine on the list.
- Walls. An eidolon opens only in a cell behind its own wall.
- Sealed chunks. Files are cut into content-defined chunks (16 KiB minimum, 64 KiB average, 256 KiB maximum). A chunk is named by an HMAC-SHA256 of its content and sealed with ChaCha20-Poly1305. A chunk that does not open, or opens to another name, is refused.
- Moves send only what changed. The sender offers chunk names, the receiver names the ones it lacks, and only those cross, already sealed. A broken move resumes from the chunks that arrived.
- A move killed at any point leaves one seat, never two and never none. The sender's copy stays closed until the person settles it with `move-settle` if it cannot tell whether the receiver took it.
- Eidolon never hides automation from a site. There are no stealth preferences and no user-agent edits. While the sign-in window is open, automation is off.
- Files for a browser pass through one folder, `$EIDOLON_HOME/files/<eid>/`. An upload must resolve to a regular file inside it.

## Limits

- The key store is a file store on this platform for now, with one owner-only file per secret under `~/.eidolon/keys`. Anyone who can read your files as you can read the keys. `eidolon machine init` says so.
- A site that ties a session to one device asks for a sign-in again after a move.
- Cells are Lima VMs, a stronger wall than the containers that Adytum can also make.
- There is no Windows or WSL driver yet.
- The sign-in window is VNC on 127.0.0.1 with a secret for one session. VNC's own authentication is weak, so both ends are loopback only.

## Where the code is

`eidolon/crates/`, members of the root workspace.

| Crate | Job |
|---|---|
| `eidolon-model` | Typed ids, the seat record, facet kinds, refusals |
| `eidolon-policy` | One seat, the holder list, the wall, what a facet may leave out |
| `eidolon-vault` | Chunks named by a keyed hash, encrypted at rest |
| `eidolon-pack` | A facet's files into chunks, the manifest and the signed head |
| `eidolon-facet` | The facet trait and the folder kind |
| `eidolon-move` | Have and want over a byte stream, the key wrapped to the receiver, the seat passed |
| `eidolon-bidi` | A WebDriver BiDi client |
| `eidolon-browser` | The browser facet, kept alive in a cell |
| `eidolon-view` | The sign-in window |
| `eidolon-mcp` | The browser's tools for agents |
| `eidolon-cli` | The `eidolon` binary |
| `eidolon-testkit` | A fake BiDi server |

## Tests

```text
npm run cargo -- test -p eidolon-cli
npm run cargo -- test -p eidolon-pack -p eidolon-move -p eidolon-vault
```

The live tests make real VMs named `adytum-test-*` in a Lima folder of their own and delete them at the end, also when a step fails. The first moves an eidolon between two cells. The second moves a folder facet and a browser facet.

```text
ADYTUM_LIVE=1 npm run cargo -- test -p eidolon-cli --test live -- --nocapture
ADYTUM_LIVE=1 npm run cargo -- test -p eidolon-cli --test live_browser -- --nocapture
```

## More

- [The guide page](../docs/guide/eidolon.md) for using it with XenoNet.
- [Release notes](RELEASES.md).
- [A skill for agents that drive the browser](../plugins/xenonet/skills/eidolon-browser/SKILL.md).
- [Adytum](../adytum/README.md), which makes the cells.
- [The design](../docs/design/cells.md).
