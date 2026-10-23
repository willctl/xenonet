# Xenolith

![Xenolith: a little less starting over](docs/showcase/xenolith.svg)

[Watch the 32-second demo](docs/showcase/xenolith-demo.mp4) · [Audit findings](docs/security-audit.md)

**Shared context for SI.**

Xenolith is shared context in plain Markdown: one folder that every session on
your computer reads and writes, whatever app or model it runs on. Any MCP client
can reach it through one local server (`xenolith`). The installer connects
Claude Code, Claude desktop, Codex and ChatGPT desktop, and packages one plugin
folder for the apps that take plugins. It needs nothing else to run; other
programs that read the folder follow [FORMAT.md](FORMAT.md).

For trusted local machines. The HTTP service does not isolate OS accounts. Read
[Security](SECURITY.md) before installing on a shared machine or enabling external model calls.

Sessions **tell the shared context what happened**, and a **curator** agent organizes it.
The server accepts each write at once into a durable inbox. The curator then
files the notes into pages, links, summaries and the activity log. It runs a
model through a command-line agent with your own sign-in. Today that agent is
the Codex CLI, with `gpt-6.1-sol` at medium reasoning by default (Status >
Models in the window changes it). Reads are instant and deterministic, because
no model runs on the read path.

Everything users run is two small Rust programs: `xenolith` (the MCP server,
the session hook, the curator, the Windows service and the installer) and
`xenolith-tray` (the tray or menu bar icon and the Xenolith window). You need
Node only to build the window. [docs/rust-plan.md](docs/rust-plan.md) covers how
the programs are built and checked.

## Install, upgrade, uninstall

### From a release archive

Releases are tagged `xenolith-v<version>`, such as `xenolith-v1.9.0`, on
[XenoNet's releases page](https://github.com/willctl/xenonet/releases). Each
has one archive per machine and a `SHA256SUMS` file:

| Machine | Archive |
| --- | --- |
| Mac with Apple silicon | `xenolith-<version>-darwin-arm64.tar.gz` |
| Mac with an Intel processor | `xenolith-<version>-darwin-x64.tar.gz` |
| Linux x64 | `xenolith-<version>-linux-x64.tar.gz` |
| Windows x64 | `xenolith-<version>-win32-x64.zip` |

An archive holds one folder with `xenolith`, `xenolith-tray`, the window's
files (`ui/`), the tray icons (`icons/`), this README, `LICENSE` and
`SECURITY.md`. The plugin, the marketplaces and the protocol are built into
`xenolith`. The machine needs no Rust, Node or checkout. Unpack it anywhere and
run the installer from the unpacked folder:

```bash
tar -xzf xenolith-1.9.0-darwin-arm64.tar.gz
cd xenolith-1.9.0-darwin-arm64
./xenolith install
```

On Windows, in a normal terminal: `tar -xf xenolith-1.9.0-win32-x64.zip`,
then `xenolith.exe install` in the unpacked folder. The installer copies the
programs into Xenolith's own folders, so you can delete the download
afterwards. Each release's notes, in [docs/releases/](docs/releases/), give the
steps for each OS: checking `SHA256SUMS`, what macOS needs when a browser
downloaded the archive, and the libraries the tray needs on Linux.

### From source

```bash
npm install
npm run install-local
npm run uninstall-local
```

`install-local` runs `npm run package`, which builds the payload into
`dist/package`: the window, the icons and the Rust programs. It builds the
programs with `cargo build --release --workspace` when cargo is installed, and
otherwise uses the ones already in `rust/target/release`. Then it runs
`xenolith install` from `dist/package`. A release archive holds the same
payload, built as [AGENTS.md](AGENTS.md#releases) describes.

### Upgrades and options

The install is idempotent. Running it again, or a later release's installer,
upgrades in place and never touches what the shared context holds. The options
are `--wiki-dir <path>`, `--port N` and `--no-approve` (after `--` with
`npm run install-local`). With `--no-approve`, Codex, ChatGPT and Claude Code
ask before each Xenolith tool call.

By default the `xenolith` tools run without asking. The installer sets
`default_tools_approval_mode = "approve"` in `~/.codex/config.toml`, and adds the
`permissions.allow` entries `mcp__plugin_xenolith_xenolith` and
`mcp__xenolith` to `~/.claude/settings.json`. Without those entries, Claude
Code denies the tools outright in `claude -p` sessions. The installer ends with a
self-test of
the server over stdio and HTTP, and of the hook under every shell the apps use.

**On Windows, run it from a normal terminal** (Win+R, `cmd`), not from the
terminal inside the Claude desktop app, the ChatGPT desktop app or the Store
version of PowerShell. Those are app packages. Windows redirects what they
create under `AppData` into the app's private storage
(`%LOCALAPPDATA%\Packages\<app>\LocalCache`), where the service, the tray at
sign-in and the other apps never see it. The installer detects an app package
and stops before changing anything.

`xenolith uninstall` (or `npm run uninstall-local`) stops the tray and removes its
start-at-sign-in entries, the plugin and marketplaces, the Claude desktop entry,
the instruction blocks and the approval settings. It leaves the shared context folder, the
programs, `config.json` and the curator's Codex sign-in in place.

After installing, sign the curator in once: tray icon > **Curator** > **Sign in
to ChatGPT for the curator...** (see [The curator](#the-curator)). Until then,
notes wait in the inbox, where you can see them.

## Moving from agent-wiki

Xenolith reads agent-wiki's wiki folder as it is. One command moves an
agent-wiki install (any version up to 1.8.0, including the Node-era ones and
the pre-1.3 `~/.agent-wiki` layout) to Xenolith. Your wiki folder stays where it
is, and Xenolith serves it as its shared context without copying it. Run the
command from the release folder (or `dist/package`), like `xenolith install`.

```bash
xenolith migrate agent-wiki          # see what it found and what it would do; changes nothing
xenolith migrate agent-wiki --run    # do it
```

The first command lists the install it found and its version, the wiki and what
it holds (pages, log days, notes waiting, dead letters, filed notes, held
changes, vectors), every registration agent-wiki made in your apps with its file
and key, what runs, and anything that would stop the move. `--json` gives the
same as data.

`--run` does seven steps and records each one in a journal in Xenolith's state
folder (`migrate-agent-wiki/`), with a copy and a SHA-256 of every file it will
change:

1. It writes the journal and the backups.
2. It pauses the curator, stops agent-wiki's tray, services and curator, and
   waits until the wiki's locks are free.
3. It writes Xenolith's `config.json` with the wiki folder and agent-wiki's
   settings (`writeMode`, `curator`, `ask`, `logs`, `hooks`, `search`).
   agent-wiki's port is not carried over. With hybrid search on and no
   credential named, it names agent-wiki's, so the stored key keeps working.
4. It moves the curator's sign-in folder to Xenolith's, so the curator stays
   signed in.
5. It runs `xenolith install --wiki-dir <your wiki>`. On Windows the same UAC
   prompt also removes the `AgentWiki` service and its folder permissions.
6. It removes agent-wiki's registrations as its own uninstaller would: the
   Claude Code plugin and its marketplace, the Codex plugin, the personal
   marketplace entry, the Claude desktop server, the instruction blocks in
   `CLAUDE.md` and `AGENTS.md`, the approval settings, and its LaunchAgents,
   user units, logon task and Run value.
7. It checks the result: the service and the tools answer on your wiki
   (`context_start`, and `wiki_start` for sessions that still use the old
   name), every page, log, note, held change and vector is still there, the
   curator is signed in, and nothing of agent-wiki's is registered.

It stops before changing anything if the wiki has a `spaces/` folder (Xenolith
keeps other spaces there), if Xenolith already has a wiki with pages, if port
47850 is taken, or if a file it must edit is not valid JSON or TOML. While
Claude desktop runs, its entry waits; quit it and run
`xenolith migrate agent-wiki --resume`. `--resume` also finishes a run that
stopped partway.

The report ends with what is left for you: restart Claude Code, Codex, Claude
desktop and the ChatGPT app, and paste the new text it prints into Claude's and
ChatGPT's personal instructions. If you edited `PROTOCOL.md`, it is kept;
`--update-protocol` renames the old tool names in it (`wiki_start` becomes
`context_start`) and keeps a copy in `.history/`.

To go back, run `xenolith migrate agent-wiki --rollback`. It undoes the steps in
reverse and starts agent-wiki again. It puts a file back only if it is still as
the migration left it, and names any file you changed since, with where its
copy is. On Windows, putting the `AgentWiki` service back needs a terminal
opened with "Run as administrator", and the report gives the command.

agent-wiki's programs stay until you finish. Once the restarts are done and no
agent-wiki program runs, `xenolith migrate agent-wiki --finish` deletes its
program, settings, state and log folders (its logs hold clipped tool arguments
that Forget cannot reach), and `~/.agent-wiki`. It never deletes the wiki. Add
`--move-wiki` to also move the wiki folder to `~/Xenolith` on the same disk.
After `--finish` there is no rollback.

## The service

The installer registers a per-machine service that serves Streamable HTTP on
`http://127.0.0.1:47850/mcp`. While the service is healthy, the plugin and
Claude Code connect to it. Otherwise each app starts its own stdio server.

- **Windows:** the service `Xenolith` runs `xenolith service`, which hosts the
  HTTP server in that one process. It starts at boot and runs as the virtual
  account `NT SERVICE\Xenolith`. That account's only rights are Modify on the
  shared context folder, the logs and the state folder (the audit chain and the [owner
  key](#the-owner-key)), and Read on `%APPDATA%\Xenolith` and
  `%LOCALAPPDATA%\Xenolith`. Registering the service takes one UAC prompt the
  first time, because the installer starts `xenolith install-service`
  elevated. `xenolith uninstall-service`, run elevated, removes it.

  Upgrades need no admin rights. An install renames the running program aside
  and puts the new one in place. The service notices the change by file
  identity and stops with an error code. The Service Control Manager's recovery
  action (restart after 2 s, 5 s, 30 s) then starts the new build.

  `install-service` also adds one ACE to the security descriptor of this service
  alone: `(A;;RPWPLC;;;<your SID>)`. It lets your account, and only yours, start
  and stop the service and query its status, so the tray can restart it. The service logs to
  `%LOCALAPPDATA%\Xenolith\logs\service.log`.
- **macOS:** LaunchAgents `dev.xenolith.server` and `dev.xenolith.curator` in
  `~/Library/LaunchAgents`, running as you, and the tray's login item
  `dev.xenolith.tray`. An install over one that used the earlier
  `dev.xenonet.xenolith.*` labels stops and removes those first, the tray's
  before any other, so two trays or two servers never run at once.
- **Linux:** systemd user units `xenolith.service` and
  `xenolith-curator.service`, running as you (`loginctl enable-linger` keeps
  them up while you are signed out).

On macOS and Linux the server runs with `--exit-on-upgrade`, so an install
restarts it the same way. The HTTP endpoints refuse any Host other than
`127.0.0.1` or `localhost`, and any cross-origin browser request, so web pages
cannot reach them, even through DNS rebinding.

## Tools

| Tool | What it does |
| --- | --- |
| `context_start` | protocol, page index, recent activity, pending notes, related pages |
| `context_search` | BM25 search over sections: each page's intro and its `##` and `###` sections, each log entry, and each note, plus the page's `aliases`. Results cite the section that matched. Filters: `scope` (`all`, `pages`, `log`, `notes`), `since` and `until` dates, and `app`. `queries` takes up to 5 other wordings, searched together and fused by reciprocal rank. With hybrid search on, results also rank by meaning (see Search) |
| `context_read` | a page, a log day, a note (`note:<id>`, pending or filed, with what the curator did with it), or a file under `pages/`, `log/` or `inbox/`, or `index.md`, `PROTOCOL.md` or `README.md`. Never `.curator/`, `.history/`, `.locks/` or `spaces/`. `slug#section` (or `section`) returns one section with the page's header line |
| `context_log` | tell the shared context what happened. The server queues it as a note for the curator, with its `source` |
| `context_upsert_page` | hand over content for one page (`append` or `replace`). The server queues it for the curator |

These names replace agent-wiki's `wiki_start`, `wiki_search`, `wiki_read`,
`wiki_log` and `wiki_upsert_page`. A call under an old name still runs the same
tool in this release, so sessions that listed the tools before an upgrade, and
protocols you edited, keep working; `tools/list` shows only the new names, and
the next release drops the old ones. Both write tools take an optional
`idempotency_key`. Without one, the server stores the same note (same app, title
and text) only once per day. Since v1.4 they also take an optional `source`:

- `user`: the user said it.
- `observed`: the app checked it.
- `external`: a web page, an email, a document or another tool's output.
- `agent`: the app's own conclusion. This is the default.

The server stamps each note with who sent it, as `by`: `local:<MCP client>`
for a process on this machine, or a token's subject (`actor:hermes`,
`machine:laptop`). The caller's `source` never says who it is. From an app,
`user` is recorded as `agent`. Only your own notes carry `user`: those you
leave with `xenolith note "<title>" [--body <text>] [--page <slug>]`
(`by: cli`) or through the window with the [owner key](#the-owner-key)
(`by: owner`), such as Revert's request to undo a change. A note file that
claims `user` without that stamp counts as `agent`.

Read results turn remote images into plain links, so no app fetches a URL from
the shared context without someone clicking it. `writeMode: "direct"` in `config.json`
restores the v1.1 behavior: immediate deterministic page and log writes, and no
curator.

### Pages as resources, and the start prompt

Every page a caller may read is also an MCP resource,
`xenolith://<kind>/<name>/pages/<slug>` (`xenolith://machine/local/pages/atlas`
is this machine's page `atlas`), read as Markdown. A client can list them, read
one, and over stdio subscribe to one: it then gets
`notifications/resources/updated` when the page changes and
`notifications/resources/list_changed` when pages come or go in its spaces. The
HTTP service is stateless and offers no subscriptions. A page the caller may
not read is "not found", like one that doesn't exist.

The `start` prompt (arguments `app`, `topic`, `space`) is what `context_start`
returns: the protocol, the space's page index, recent activity and notes not
yet filed. Clients that put a prompt in front of a conversation can use it in
place of the tool call. The read-only server that Ask uses offers tools only.

## Spaces and access

Shared context is kept in spaces. The folder itself is this machine's space,
`machine:local`. Every other space has the same layout, in
`spaces/<kind>/<name>/` inside it ([FORMAT.md](FORMAT.md#spaces)):

| Space | Holds |
| --- | --- |
| `project:<name>` | one project's context |
| `identity:<name>` | what belongs to one identity: an account, a persona |
| `actor:<slug>` | an actor's own notes, such as an always-on agent's |
| `machine:<name>` | one machine's context; `machine:local` is this one |

Every tool takes `space`. Without it, a call works in `machine:local`, or, for
a token's holder, in the first space it may use. `context_start` names the
space and lists the others the caller can use. A note logged to a space that
doesn't exist yet starts it; a read never does. Each space has its own inbox,
pages, log, history and curator, and reads in `machine:local` never reach into
`spaces/`.

Access comes in four levels, each including the ones before it:

| Level | Lets a caller |
| --- | --- |
| read | `context_start`, `context_search`, `context_read` |
| append | `context_log`, and `context_upsert_page` with `mode: "append"` |
| write | `context_upsert_page` with `mode: "replace"` |
| admin | start a space |

Local callers have admin in every space: stdio servers, and HTTP requests that
come straight from a process on this machine ([who is local](#who-is-local)).
Anyone else gets nothing until you grant it:

```bash
xenolith grant actor:hermes ctx:append:by:hermes --days 30
xenolith grant client:phone ctx:read:identity:adal
xenolith revoke actor:hermes
xenolith access
xenolith spaces add project:atlas --identity adal
```

A scope is `ctx:<level>` or `ctx:<level>:<selector>`. A selector is
`key:value` terms that must all hold, joined with `+` inside a scope. `all`
covers every space, and a value may end in `*`. The keys are `project`,
`identity` (an identity's own space and the projects that belong to it),
`machine` and `by` (an actor's own space: `by:<slug>` names the actor). A
subject is `actor:<slug>`, `client:<id>` or `machine:<name>`.
A grant expires (after 30 days unless you say otherwise), `revoke` takes effect
at once, and a grant that names one space exactly starts it. A token reaches
this machine's own space, `machine:local`, only when its grant and its scopes
both name it exactly (`ctx:read:machine:local`): `all` and `machine:*` don't
reach it, since it holds your own notes and the questions you ask in the
window. The policy is `access.json` in Xenolith's config folder, never in the
shared context folder.

Reads return content only: pages, the log, notes not yet filed, and the
top-level `index.md`, `PROTOCOL.md` and `README.md`. The curator's state
(`.curator/`: held changes, journals, audit records, Ask), `.history/`,
`.locks/` and other spaces' folders stay out of reach of every caller, whatever
path it names or a link inside points to.

### Who is local

Local is any process on this machine, not only your own apps. A request is
local when it reaches the service's loopback port directly, names `127.0.0.1`
or `localhost` as its Host, and carries no `Forwarded`, `X-Forwarded-*` or `Via`
header. Tokens scope remote access and other apps' access. They don't separate
your own agents from each other.

| Endpoint | Who |
| --- | --- |
| `/mcp` | a local request without a token, or a token |
| `/sync/v1` | a token, always, even from this machine |
| `/status`, `/ui`, `/api` | local requests only, never a token |
| `/health` | anyone: name, version, build, pid and uptime, nothing private |

A forwarder that adds none of those headers, such as `ssh -L`, socat or a plain
TCP tunnel, looks local. If anyone reaches the port that way, take local access
away and give each client its own token:

```bash
xenolith access local none
xenolith grant client:laptop ctx:append:project:atlas
xenolith token create client:laptop
```

`xenolith access local read` (or `append`, `write`, `admin`) gives some of it
back. In `access.json` this is `"local"`; a value Xenolith doesn't know counts
as `"none"`.

### The owner key

Approving a held change or a proposal, Undo, Revert, the Approvals setting and
Forget are yours alone, so they need more than a local request: the owner key.
Each time the service starts it writes a new random key to `runtime/owner.key`
in Xenolith's state folder, readable by you alone (on Windows, by the user who
installed the service). The tray reads it, checks that the service answering on
its port has the pid the file names, and opens the window with the key in the
URL fragment, which never reaches the server or its logs. The window sends it
back in an `X-Xenolith-Owner` header. A process that sends only `X-Xenolith: ui`
can read the window's API but not decide anything. `xenolith held approve`
reads the key the same way and asks the service to decide.

Every decision goes in the audit chain with who made it: `owner` for the
window, `cli` for the terminal. Refused attempts go there too.

What this guarantees: a decision needs an explicit read of a file only you can
read, and the audit chain records it. A sandboxed agent that can't read
Xenolith's state folder can't approve its own change. An agent running
unsandboxed as you can read the file, as it can any of your files.

### Tokens

A grant does nothing until its subject holds a token. Tokens are
`xenolith_` and 64 hex digits; Xenolith keeps only their SHA-256, and shows a
token once:

```bash
xenolith token create actor:hermes --label "Hermes on the server" --days 90
xenolith token create client:phone --scope "ctx:read:project:atlas"
xenolith token revoke tok_1a2b3c4d5e6f
```

A token's scopes default to what its subject is granted when it is made. What
a token may do in a space is the lower of its own scopes and its subject's live
grants there, so a token never reaches past what you granted, and revoking
either one takes effect on connections already open.

- **Over HTTP** a token travels as `Authorization: Bearer`, at `/mcp` and `/sync/v1` only. The
  service acts as an OAuth 2.1 resource server the way MCP's authorization spec
  describes: its metadata is at `/.well-known/oauth-protected-resource`, a
  missing or bad token gets a 401 with a `WWW-Authenticate` challenge, and a
  call the token's scopes can't reach gets a 403 with `insufficient_scope` and
  the scope it needs. A call its scopes reach but its grants don't is refused
  with the same answer whether the space exists or not. A token in a URL is
  refused, and a token at `/status`, `/health`, `/ui` or `/api` gets a 403,
  since those belong to this machine. Without a token, a local request is this
  machine's own ([who is local](#who-is-local)), and a forwarded one gets a 401.

  After 20 bad tokens in a minute from one source, every request with a token
  from it gets a 429 for a minute, and the audit chain records the block. A
  source is the connection's address and, behind a proxy, the client the proxy
  last named (`Forwarded: for=`, `X-Forwarded-For`), so one caller guessing
  through a gateway doesn't lock out the others.

  Behind a proxy, set `"publicUrl"` in `config.json` (such as
  `"https://studio.example/xenolith"`). Forwarded callers then see it in the
  resource metadata and the `WWW-Authenticate` challenge, with the metadata at
  `https://studio.example/.well-known/oauth-protected-resource/xenolith/mcp`
  as RFC 9728 places it. This machine's own callers see the loopback address.
- **Over stdio**, `XENOLITH_TOKEN` in the server's environment makes it work
  as the token's holder, for an actor started on this machine with its own
  token. A token Xenolith doesn't know stops the server from starting.

The secret guard refuses tokens in notes and pages.

### The audit chain

Grants, revocations, tokens, refusals, writes and new spaces each add one JSON
line to `audit/chain.jsonl` in Xenolith's state folder: when, what, who (`you`,
or a token's subject and id), and the space. A write records the note's id and
a hash of what was written, never the text.
Each line carries the SHA-256 of the line before it, so a line changed, removed
or put in between breaks the chain at that point:

```bash
xenolith audit verify
xenolith audit tail 20
```

`verify` names the first line that doesn't follow and exits 1. The chain shows
tampering but can't prevent it: whoever can write the file can rewrite all of
it. Keep the last hash `verify` prints somewhere else to pin it. The curator's
own record of each filed note stays in `.curator/audit/` inside each space.

Each line is on disk before the call it records returns. A line that can't be
written is never silent: the service counts it, `/status` reports "the audit
chain can't be written" and turns degraded, and `xenolith status` says the same
and exits 1:

```bash
xenolith status          # the service's health, and whether the chain holds and can be written
```

### Sync between machines

Each space has one curator: the machine whose Xenolith files its notes into
pages. A space you start is curated where you start it. Another machine can
**follow** it: it keeps a copy, where its sessions read pages and leave notes
as usual, and its own curator leaves that space alone. Once a minute (or on
`xenolith sync`) it pushes the notes waiting there to the curator, as entries,
and pulls the curator's pages, its removals and the last 7 days of its log
back, so `context_start` on the copy shows recent activity.

On the curator, grant the other machine and make it a token:

```bash
xenolith grant machine:laptop ctx:write:project:atlas
xenolith token create machine:laptop
```

On the other machine, name it, add the curator as a peer (the token comes on
stdin, so it never shows in a process list), and follow the space:

```bash
xenolith sync name laptop
xenolith sync peer add studio https://studio.example/xenolith < studio.token
xenolith sync follow project:atlas studio
xenolith sync
```

A peer's URL is the base of its Xenolith service: `https://` anywhere, or
`http://` only to this machine, since a token never crosses a network in the
clear. The base may carry a path (`https://box.example/xenolith`): the sync
routes go under it, at `<base>/sync/v1/...`. Xenolith serves them at the root of
its own port, so whatever forwards an address to it (a reverse proxy, a tunnel)
must strip its own path prefix before passing a request on, and send it to
`127.0.0.1:<port>` with that as its Host. The token is kept in
`peers/<name>.token` in the config folder, readable by you alone from the
moment it exists: mode 0600, or on Windows a protected ACL for you and SYSTEM.
`sync.json` and `access.json` are written the same way, also readable by the
Windows service, which checks tokens and curators with them. One request to a
peer may take 90 seconds; a peer behind a slow forwarder may need longer:
`xenolith sync timeout <seconds>` (`timeoutSeconds` in `sync.json`).

**Who the curator is.** Each curated space has a random curator id, made the
first time a follower asks (`.curator/sync/curator.json` in the space). A
follower pins it in `sync.json` at its first pull. If the peer's address later
answers as another curator (two machines curating one space, or the curator set
up again), the follower refuses to push or pull and says so in `xenolith sync`,
`xenolith sync status` and `/status`. When the new curator is the right one,
`xenolith sync follow <space> <peer>` again accepts it.

**What travels.** The curator takes entries at
`/sync/v1/spaces/<kind>/<name>/entries`: a note needs append, a replaced or
removed page needs write. Each entry is taken once, however often it is sent,
and is stamped as the pushing token's subject's (`by: machine:laptop`), so a
`user` source from the other machine counts as `agent` here. A page replaced on
the other machine carries the version it was written against. When the
curator's page has changed since, the push gets **409** and the write becomes a
**proposal**: a change that waits for your OK even when approvals are
automatic, listed by `xenolith held --space <space>` and decided there with
`xenolith held approve|deny <batch>:<index> --space <space>` (or through the
window's API with `space` and the [owner key](#the-owner-key)). Approving it
applies it only if the page hasn't changed again.

- **Edits in a copy.** A page changed in the follower's copy since the last
  pull (by hand, or by a session writing directly) becomes an entry: a replace
  against the version last pulled. The copy then shows the curator's page again
  until the curator files the change; the edit stays in `.history/`.
- **Removals.** A page removed in a copy is sent as a removal against the
  version last pulled. The curator removes it too when that is still its
  version (keeping it in `.history/`), and makes it a proposal otherwise; until
  then the next pull brings the page back. A page removed on the curator, by
  any means, is listed as removed for 30 days, and each follower removes its
  copy, keeping it in `.history/`.
- **Batches.** Entries go 100 at a time, one batch a pass. A backlog over 1,000
  is pushed batch after batch until it is drained.
- **Failures.** An entry the curator refuses for good, because the token may not
  do it (403) or the entry is invalid (400), moves to the copy's
  `.curator/sync/dead/` with the reason on its first line and is never sent
  again. `xenolith sync status` and `/status` count them. A push that fails for
  any other reason leaves its entries to the next pass, and the pull still
  runs. Only a refused token (401, or 403 for the whole request) stops both. A
  page that disappears between the list and its fetch is skipped.

Nothing written on either machine is lost: each pushed note moves to
`.curator/sent/` with what the curator said, and each pull keeps the page it
replaces or removes in `.history/`. Log lines written directly in a copy stay
after the curator's when its log is pulled. A machine that only follows a
space sends anyone who asks it for that space on to the curator (421, with the
curator's URL when it is an `https://` one). Both machines record each sync in
their audit chains.

## The curator

`xenolith curator` runs as you, never as the service. It runs the Codex CLI
with your own sign-in (a ChatGPT account), and the service account must not
have that sign-in. Other command-line agents are not supported yet. On Windows the tray hosts the curator and restarts it when
`xenolith` is replaced. On macOS and Linux, a launchd agent or a systemd user
unit runs it. It never reads Codex's credential files.

- **Isolation.** It runs `codex exec` in its own `CODEX_HOME`
  (`%LOCALAPPDATA%\Xenolith\curator\codex-home`) with `--ignore-user-config`,
  `--ephemeral` and `--sandbox read-only`. It disables plugins, hooks, apps,
  memories, shell and multi-agent tools, and turns web search off. It needs a
  separate home because Codex always loads `~/.codex/AGENTS.md` (our pointer) and
  has no switch to stop that. A run with that file loaded tried to reach the `xenolith`
  tools. The separate home needs its own one-time `codex login` (tray > Curator >
  Sign in).
- **Structured output.** The model returns an edit plan (`--output-schema`) with
  three parts:
  - for each page, `create` (full body), `patch` (exact-match find-and-replace
    edits) or `replace`, each with the page's base hash;
  - for each note, a disposition (integrated, log_only, duplicate, ignored) and
    a reason;
  - log entries.

  The curator validates the plan: ids, slugs, hashes, edits that match exactly
  once, and the secret guard on everything the model wrote. If validation fails,
  it asks the model once for a repair. Then it applies the plan
  deterministically.
- **Pages hold the current state.** When a fact changes, the curator updates it
  in place and adds one line to the page's `## History` section: the date, what
  changed and the old value. Verification steps, test counts and timings go to
  the log entry, not the page. A change that would take a page over the soft cap
  (`pageCapChars`, 12,000 characters) gets one repair round, which asks the model
  to move a section to a new, linked page instead. The curator never cuts
  content to fit. It names any page that does not fit in the model's context
  (`contextChars`) in the audit record and in `/status`, and never skips a page
  silently.
- **Trust gate.** Code, not only the prompt, flags a page change in two cases:
  - any note behind it is `external`;
  - its notes are not all yours (`user`, stamped by the server, not claimed by
    an app), and it changes a `preference` page, adds an instruction-like line
    ("Always ...", "from now on", "ignore ..."), or adds a link to anything but
    this machine.

  Flagged changes go to `.curator/held/<batch>.json`. The **Approvals** setting
  at the top of the window's Inbox decides what happens next:
  - **Automatic** (the default): the curator applies the change in the same
    step that files the batch, and the tray shows the notification "Xenolith
    applied a change". Clicking it opens the Inbox. The tray remembers what it
    announced in `tray-state.json` next to its `tray.ini`, so a restart repeats
    nothing and misses nothing. The Inbox lists the changes from the last 7 days
    under "Applied automatically", each with its reason, its diff and an
    **Undo** button. Undo puts the page back if nobody has edited it since. A
    cleanup that touched several pages (a split) is undone as a whole. A change
    that no longer fits its page (the page changed since) waits under "Needs your
    OK" instead, and the log says why.
  - **Ask me first**: the change waits, the rest of the batch applies, and the
    log says the change is waiting. The Inbox shows each waiting change with its
    reasons, its notes and a diff, and offers **Approve**, **Reject** and **Send
    back**. Send back returns its notes to the curator. `/status` reports "N
    change(s) need your OK", so the tray shows that something needs attention.
    Switching back to Automatic applies everything that is waiting. Without a
    tray, `xenolith held` lists what waits, each as `<batch>:<index>`, and
    `xenolith held approve|deny|refile <batch>:<index>` decides one.

  Deciding a change, Undo, Revert, the Approvals setting and Forget need the
  [owner key](#the-owner-key). An approver never approves a change made from a
  note it submitted itself.

  The setting is `approvals` (`auto` or `manual`) in
  `.curator/settings.json`, so it travels with the folder. A file that cannot be
  read or understood counts as Ask me first, and `/status` names the problem.
  Forget requests always wait for your OK. If an installed folder's `PROTOCOL.md` was never edited, an
  install replaces it with this release's copy.
- **Cleanup schedule.** On a schedule, daily at 03:00 by default, the curator
  reviews each page that changed since its last review, one page at a time when
  it has nothing to file. It also reviews a page soon after the page grows past
  the size cap. A scheduled time missed while the computer slept is caught up
  when the curator is next idle. The review sees the
  index, the recent log, and what code checks by itself: broken `[[links]]`,
  orphan pages and size. It looks for contradictions, stale "pending" items,
  time-relative wording, facts the log has since changed, an outdated summary,
  work narration and pages to split.

  With Automatic approvals, the curator applies each cleanup, notifies you and
  lists it for Undo, like any flagged change. A cleanup goes in whole or not at
  all, and what it wrote counts as reviewed, so the page is not reviewed again
  until something else changes it. With Ask me first, the cleanup
  waits in the Inbox as "Proposed cleanup", with a diff, **Approve** and
  **Approve all cleanups**. The first pass after install tidies the pages
  written before these rules. `xenolith curator --lint [slug...]` runs a
  review now.

  Change the schedule in the window's Inbox, under the Approvals switch:
  **Daily** or **Weekly** at a time you pick, **Custom** for any cron schedule
  (with its next three times shown as you type), or **Off**. It is
  `cleanupSchedule` in `.curator/settings.json`: a five-field cron expression
  in the computer's local time (minute, hour, day of month, month, weekday;
  for example `0 3 * * 1-5` for weekdays at 03:00, or `0 */6 * * *` for every
  six hours), `@daily`-style shortcuts, `daily`, `weekly` (Mondays at 03:00)
  or `off`. A schedule that cannot be read runs the default, and `/status`
  names the problem. `curator.lint: "off"` in one computer's `config.json`
  turns cleanups off there, whatever the folder's schedule says.
- **Revert.** Each curator page change in the window's Activity has a **Revert**
  button. It restores the `.history/` copy if the page is still exactly what
  that batch wrote, which it checks against the audit record's hashes.
  Otherwise it offers to ask the curator, in a note from you, to undo the change
  on the current page.
- **Forget.** The window's Status tab has a Forget box. It lists every copy of a
  piece of text, with the text masked: pages and their `.history/`, filed notes,
  audit records, held changes, Ask, the log, the inbox and the request logs.
  Then it redacts every copy and leaves a tombstone in `.curator/forgotten/` that
  records how much it redacted, never the text. `xenolith forget "<text>"
  [--yes]` does the same from a terminal. When your own note asks Xenolith to
  forget something (`xenolith note "FORGET ..."`, a note with source `user`),
  the curator records a forget request instead of copying the text anywhere,
  and the Inbox asks for your OK. An app's request to forget is not recorded,
  even when it says you asked. Forget does not rewrite a git history of the folder.
- **Nudge before stopping (opt-in).** The plugin also registers a Stop hook
  (`xenolith hook stop`, for Claude Code and Codex). With `"hooks": {"nudge":
  true}` in `config.json`, the hook asks the model to log what is worth keeping
  before it stops. It asks once per session, and only when the transcript is
  long (`nudgeMinLines`, 30) and has no `context_log` call. Without that setting it
  does nothing.
- **Sources stay reachable.** Each log entry ends its header with a
  `sources: note:<id>` line. `context_read("note:<id>")` opens that note wherever
  it is, in the inbox or in `.curator/done/`, with the curator's disposition and
  the pages it changed. Search covers filed notes and ranks them below pages and
  the log. The window opens a filed note from search or from an activity entry.
- **Concurrency.** Commits take the folder's write lock and re-check every base
  hash. If someone edits a page in the meantime, the curator plans again on the
  new content, up to 5 times, and never loses the edit. Commits go through a
  write-ahead journal, so after a crash at any point the curator completes the
  commit on restart, exactly once.
- **Throughput.** The curator waits until 20 s pass with no new note, but never
  more than 2 min, then files batches of up to 12 notes. Failures back off exponentially,
  from 1 min to 1 h. After 5 failures a note becomes a dead letter, shown in the
  tray and in `context_start`. From the tray you can **Retry** it or **File as
  sent**. When the curator is signed out or hits a usage or rate limit, notes
  wait without using up attempts.
- **Audit.** The curator writes `.curator/audit/YYYY/MM/<id>.json` for every
  filed note. It records what changed, why, the base and new hashes, the
  `.history/` copy, the model and the batch.
- **Models.** The window's Status tab has a **Models** card. For the curator
  and for Ask, pick a model from your account's list and a reasoning
  level, or keep the default: `config.json`, then `gpt-6.1-sol` with medium
  reasoning for the curator, and the curator's model with low reasoning for
  Ask. The list is Codex's own list of the models your account offers
  (`models_cache.json` in the curator's Codex home), which the curator copies
  into `.curator/models.json` so the window can show it. A model or reasoning
  level the account does not offer is refused, with what it does offer. A
  chosen model the account stops offering shows in `/status`, the window and
  the tray. A change applies from the next run, with no restart. When Codex
  refuses a model, the curator waits instead of failing notes, and tries again
  as soon as you choose another. From a terminal, `xenolith models` shows
  what each one uses and the account's models;
  `xenolith models set curator gpt-6-astra --effort high`,
  `xenolith models set ask --effort medium` and
  `xenolith models reset ask` change them (`--force` saves a model the
  list does not have). The choice is `models` in `.curator/settings.json`.
  The tray's Curator menu shows both models.
- **Settings.** The `curator` object in `config.json` (see
  [What goes where](#what-goes-where)) takes `model` (`gpt-6.1-sol`) and
  `reasoningEffort` (`medium`), the defaults under the window's choice, and
  `debounceSeconds`, `maxWaitSeconds`, `batchMax`, `batchChars`,
  `contextChars`, `pageCapChars`, `lint` (`off` turns cleanups off on that
  computer), `maxAttempts`, `timeoutSeconds`, `codexPath` and `codexHome`. To pause or
  resume the curator, use the tray menu, or create or delete `.curator/paused`.
- **Command line.** `xenolith curator --once` (file what is due, then exit),
  `--lint [slug...]` (review pages now), `--retry-dead`, `--file-raw-dead`,
  `--login-status`, `--asks-only`.

Two parts of the folder are the curator's. `inbox/<id>.md` holds pending notes as
plain Markdown, and you can drop notes there by hand too. `.curator/` holds
keys, state, journal, done, audit, status, and `asks/` for
[Ask](#ask-agentic-search).

## Search

Plain search is BM25 over sections and needs no model; it stays the default.
Two additions make it find a page by other words than its own:

- **Aliases.** The curator writes an `aliases` line on each page: the other
  words a person would search for it with ("Portugal, travel" on a Lisbon
  trip). Scheduled cleanups add them to existing pages. Search weights them
  like tags.
- **Hybrid search (opt-in).** Each section is embedded once by a hosted model
  through OpenRouter, and a search combines BM25 with the cosine similarity
  of each file's best section (a convex combination of normalized scores).
  Questions lean on meaning (weight 0.9) and a few keywords on words (0.5).
  A query that names a date stays BM25, because embeddings know no dates. The
  query's embedding has 1.5 s; after that, or with no key or no network, the
  search is BM25 alone.

Turn hybrid search on in `config.json`:

```json
"search": { "embeddings": { "enabled": true, "credential": "Xenolith/OpenRouter" } }
```

The API key is read from `OPENROUTER_API_KEY` or, by the `credential` name,
from Windows Credential Manager (a generic credential), the macOS keychain (a
generic password) or the Linux secret service. Xenolith never stores it.
`model` defaults to `google/gemini-embedding-2`; `weight`, `keywordWeight`
and `queryTimeoutMs` adjust the rest. Requests ask OpenRouter for providers
that neither collect nor retain the text, so the pages' sections and the
queries go to OpenRouter and the model's provider under that policy. The
curator embeds new and changed sections when idle;
`xenolith embeddings status` and `xenolith embeddings sync` do it by
hand. Vectors live in `.curator/vectors/<model>/`, and Forget deletes them
all so nothing of a forgotten text survives there. On Windows the service runs
as its own account, which cannot read your credentials, so the tray hands it
the key over a local pipe that only you and the service can open; the service
keeps it in memory, never on disk. The tray first checks that the pipe belongs
to the service's account and to the process `/status` names, so no other
program can receive the key. `/status` shows `embeddings.keyAvailable`;
without the tray running, searches through the service stay BM25.

[docs/search-plan.md](docs/search-plan.md) has the measurements: on the hard
eval set, plain search found 26 of 55 answers first and hybrid search 48 (exact
McNemar p < 0.001), and notes routed to their page went from 4 of 12 first to
12 of 12.

## Ask (agentic search)

The window's **Ask** tab (and the first row of every search) hands a question
to an agent that searches and reads the shared context for you, then answers with the
files it relied on. It runs in the curator process, as you, for the same reason
the curator does: the service account must not have your model sign-in.

- **The agent.** Ask uses the curator's isolated `codex exec`, with the same
  `CODEX_HOME`, the same features off and no shell. It gets exactly one MCP
  server, `xenolith serve --read-only`. That server offers `context_search` and
  `context_read` and nothing that writes. It does not even create the skeleton,
  break locks or refresh the index. Codex's `enabled_tools` also lists only those
  two. The prompt carries the page index and up to three earlier turns of the
  conversation. The answer comes back as JSON: `answer` in Markdown with
  `[[links]]`, `found`, and `sources` with a supporting quote each.
- **The queue.** The service writes `.curator/asks/<id>/ask.json`. The worker
  claims it with an exclusive create, and it runs at most 2 questions at a time.
  As Codex reports each step (searched for X: N results, read page Y, a remark),
  the worker appends it to `events.jsonl`. At the end it writes `result.json`.
  The window polls `/api/ask?id=&after=N`. **Stop** writes a `cancel` file. A
  question that no worker picks up within 10 minutes expires. If a question's
  worker stops sending heartbeats for 90 s, the question is marked interrupted.
  The worker keeps questions for 14 days, at most 200. While the worker's
  heartbeat (`asks/worker.json`) is stale, the service refuses new questions
  with a 503 and the reason. It also refuses a question that contains a secret.
- **Settings.** The `ask` object in `config.json` takes `model` (default: the
  curator's) and `reasoningEffort` (`low`), the defaults under the window's
  choice (Status > Models), `timeoutSeconds` (180),
  `maxConcurrent` (2), `queueSeconds` (600), `keepDays` (14) and `keepMax` (200).
- **Request log.** The curator writes `kind: "ask"` lines (start, done with
  result, searches, reads, sources, usage). The agent's own tool calls appear as
  `proc: "ask"` lines tagged `ask: <id>`.

## Tray app

`xenolith-tray` runs one instance per session, on Windows (notification area),
macOS (menu bar) and Linux (StatusNotifierItem). It talks to the service only
over HTTP (`GET /status`) and shows healthy, degraded (backlog, failed notes,
curator signed out or erroring, server errors) or down. **Left click** opens the
Xenolith window (below). **Right click** shows the menu:

- Open Xenolith;
- status (version, uptime, queue, last write, reasons);
- open the folder, `index.md` or the logs;
- recent activity;
- pause or resume the curator;
- Curator (sign in, retry or file failed notes, restart, its log);
- Start at sign-in;
- copy MCP URL;
- restart service;
- quit.

Starting the tray again while it runs opens the window. The icon is a dog-eared
page with a speech-bubble tail, because what the apps say becomes pages. It
is indigo when healthy, has an amber dot when it needs attention, and is grey
with a red dot when the service is down. `assets/icon/xenolith-16.svg` is
fitted to the 16 px grid that the tray uses at 100% scaling, and
`xenolith.svg` draws 20 px and up. `npm run icons` builds `dist/icons/*.ico`
(16 to 256 px) and `dist/icons/preview.png`, which shows every state and size on
light and dark taskbars.

**The Xenolith window.** The window is a React app (`ui/`, built by
`npm run build`) that the service serves at `http://127.0.0.1:47850/ui/`. The
tray shows it in its own small window: WebView2 on Windows, WKWebView on macOS
and WebKitGTK on Linux, with its own data folder of a few MB. It opens above the
tray, with the [owner key](#the-owner-key), so it can approve changes. Opened
anywhere else, such as in a browser, it can read but not decide. Links that leave `127.0.0.1` open in your browser. When you close the
window, the tray hides it and frees it after five idle minutes, so the tray
stays small: about 11 MB with the window closed, measured on Windows (run count
not recorded). The window has a search box and five tabs:

- **Search** (`/` or Ctrl+K, then arrow keys and Enter) covers pages, notes
  waiting for the curator and the activity log, with highlighted snippets. The
  first row asks the shared context instead. The window selects it when the query reads
  like a question, and Ctrl+Enter picks it from anywhere in the box.
- **Ask** sends a question to an agent that searches and reads the shared context (see
  [Ask](#ask-agentic-search)). Each search and read appears as it happens, then
  the answer with numbered sources. Follow-ups keep the conversation, and the tab
  lists recent conversations.
- **Pages** filters by type and renders each page from Markdown, with clickable
  `[[links]]`, **Linked from** (backlinks), **Links to**, earlier versions, and
  actions that copy the `[[link]]` or the file path.
- **Activity** shows the log by day (what happened, page changes, or
  everything), with each entry's app and linked pages, and its full text on a
  click.
- **Inbox** lists notes waiting for the curator (waiting, retrying, or failed
  with the last error), and has pause and resume.
- **Status** shows the service, the curator (model, sign-in, last run), the
  queue and requests.

The window follows the system's light or dark theme. It reads a small JSON API
on the same port: `/api/status`, `/api/pages`, `/api/page?slug=`,
`/api/search?q=`, `/api/activity?days=`, `/api/log?date=`, `/api/inbox`,
`/api/held`, `/api/asks` and `/api/ask?id=&after=&thread=1`. The writes are
`POST /api/curator {"paused": bool}`, `POST /api/ask {"question", "parent"?}`,
`POST /api/ask/cancel {"id"}`, `/api/models`, `/api/settings`, `/api/held`,
`/api/revert` and `/api/forget`. Each write needs the `X-Xenolith: ui` header,
which another site's page cannot send.

| Call | Needs |
| --- | --- |
| reads, Ask | `"local"` read or more, or the owner key |
| pause the curator, models, cleanup schedule | `"local"` admin (the default), or the owner key |
| decide, Undo, Revert, the Approvals setting, Forget | the owner key |

The API takes local requests only ([who is local](#who-is-local)), refuses the
same Hosts and Origins as `/mcp`, and any request a browser marks as sent by
another site (`Sec-Fetch-Site` other than `same-origin` or `none`). The window's pages get a strict CSP, and nothing in a
page runs: raw HTML shows as text, and only `http`, `https` and `mailto`
links work. To work on the window, run `npm run ui:preview`. It serves a
snapshot copy of your shared context folder on port 47859 (`http://127.0.0.1:47859/ui/`) and does
not touch the real one.

**Start at sign-in.** On Windows the tray has two independent per-user entries,
and neither needs admin rights: the Task Scheduler logon task `XenolithTray`
(runs as you, normal priority, no time limit) and `HKCU\...\Run\XenolithTray`.
Either one starts the tray, and whichever starts second exits. There are two
because the Run value alone has been seen to disappear overnight, removed by
something outside Xenolith. On macOS the entry is a
LaunchAgent, and on Linux it is an XDG autostart entry. The tray checks its
entries every 2 minutes and writes any change, with the time window, to
`logs/tray.log`. The log also records when the tray started and which entry
started it. The tray never re-adds an entry by itself. Restoring its own
persistence is what malware does, and endpoint security treats it that way.
**Start at sign-in > Repair** (`xenolith-tray --do repair-autostart`) or the
installer registers the entries again.

**Tray icon.** Windows 11 puts new tray icons in the hidden overflow (^), and
only you can pin one. To pin it, open Settings > Personalization > Taskbar >
Other system tray icons, and turn Xenolith on. Explorer keeps that choice
itself, so the installer only reports where the icon is.

For headless use, the tray takes these flags:

- `--selftest` prints the state, the start-at-sign-in entries, whether it can
  use the service's owner key, and the menu.
- `--do <action>` runs one menu action: `open-ui`, `open-wiki`, `open-index`,
  `open-logs`, `pause`, `resume`, `copy-url`, `sign-in`, `retry-dead`,
  `file-raw`, `restart-service` or `repair-autostart`.
- `--quit` stops the running tray.
- `--window` opens only the window.

## Request log

Every HTTP request, every tool call (stdio and HTTP) and every curator run is
one JSON line in `logs/requests-YYYY-MM-DD.jsonl` (`%LOCALAPPDATA%\Xenolith\logs`
on Windows). Each line has the time, request id, process, transport, client name
and version, `app`, tool, a short redacted argument summary, duration, result
and error. The client name and version come from `initialize`. Over stateless
HTTP, they come through an `Mcp-Session-Id` that the server hands out at
initialize. The same guard that refuses secrets redacts them, before truncation.
Several processes append at once, so each line is a single append-mode write
under 4 KB, which is atomic on Windows and POSIX. The files roll over daily, and
by size within a day. Retention is 30 days (`logs.retentionDays`).

## What goes where

Xenolith's own files live in the platform's standard folders, never in a dot
folder in your home. On Windows:

| Path | What |
| --- | --- |
| `%APPDATA%\Xenolith\config.json` | `wikiDir`, `version`, `httpPort`, `writeMode`, `curator`, `logs`, `publicUrl` |
| `%APPDATA%\Xenolith\access.json`, `sync.json`, `peers\` | who may use which space, tokens' hashes, sync peers and their tokens (yours alone) |
| `%LOCALAPPDATA%\Xenolith\runtime\` | `xenolith.exe` and `ui/` (the Xenolith window) |
| `%LOCALAPPDATA%\Xenolith\service\` | the service's ini |
| `%LOCALAPPDATA%\Xenolith\tray\` | `xenolith-tray.exe`, its ini, its logon-task definition, icons and the window's data folder (`webview\`) |
| Task Scheduler `\XenolithTray`, `HKCU\...\CurrentVersion\Run\XenolithTray` | start the tray at sign-in |
| `%LOCALAPPDATA%\Xenolith\curator\codex-home\` | the curator's own Codex home and sign-in (Local, not Roaming: it stays on this PC) |
| `%LOCALAPPDATA%\Xenolith\logs\` | `service.log`, `curator.log`, `tray.log`, `requests-*.jsonl`, `http-sessions.json` |
| `%LOCALAPPDATA%\Xenolith\state\` | `install-state.json` (what the installer created, for uninstall), `audit\chain.jsonl`, `runtime\owner.key` (the [owner key](#the-owner-key), new at each service start), `migrate-agent-wiki\` (the journal, backups and log of a [move from agent-wiki](#moving-from-agent-wiki)) |
| `%LOCALAPPDATA%\Xenolith\marketplace\` | local marketplace (Claude and Codex formats) and the rendered plugin |
| `%LOCALAPPDATA%\Xenolith\paste-into-app-settings.md` | the pointer text, for app personal-instruction settings |
| `~/Xenolith/` | the shared context (a folder of your documents, not app data) |
| `~/.agents/plugins/marketplace.json` | ChatGPT and Codex personal marketplace entry (Codex's folder) |
| `~/.codex/config.toml` | `[plugins."xenolith@personal".mcp_servers.xenolith]` auto-approval |
| `~/.codex/AGENTS.md`, `~/.claude/CLAUDE.md` | managed `XENOLITH` instruction block |
| `claude_desktop_config.json` (MSIX: `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude`) | `mcpServers.xenolith` for ordinary Claude desktop chats |

On Linux the same files go to the XDG folders (`~/.config/xenolith`,
`~/.local/share/xenolith`, `~/.local/state/xenolith` with `logs/`,
`~/.cache/xenolith`). On macOS they go to `~/Library/Application Support/Xenolith`,
`~/Library/Logs/Xenolith` and `~/Library/Caches/Xenolith`. On both, a
set `XDG_*` variable wins. `XENOLITH_HOME=<dir>` puts everything in one folder
instead. The tests and `ui:preview` use it. Each of
`XENOLITH_{CONFIG,DATA,STATE,LOG,CACHE}_DIR` sets one kind of folder. The
service gets these variables from its ini, because its own profile is not yours.

Xenolith looks for the shared context folder in this order: the `XENOLITH_DIR`
environment variable, then `wikiDir` in `config.json`, then `~/Xenolith`. The
installer seeds `PROTOCOL.md` into a new folder. In an existing one it replaces
the file only when it is an unedited earlier default, so it never overwrites
your edits.

Before it changes an existing config file, the installer copies it once to
`<file>.bak-xenolith` and never overwrites that first backup. Then it merges
its changes in and keeps the file's line endings.

## With XenoNet (optional)

Xenolith runs on its own, and nothing in it depends on XenoNet. If you use
XenoNet too, it does two things with Xenolith:

- **It shows the shared context.** XenoNet's map reads the folder (pages, the
  log and the inbox, as [FORMAT.md](FORMAT.md) describes them) and shows each
  session's writes next to the pages they touch. It only reads; Xenolith's
  server does all the writing.
- **It can carry sync between machines.** A XenoNet network can pass one
  host's Xenolith sync routes (`/sync/v1/`) to another host, which then follows
  the curator at a loopback address XenoNet gives it, with a path prefix
  XenoNet strips before it passes a request on. To Xenolith that is an ordinary
  [peer](#sync-between-machines): the request carries Xenolith's own token, a
  forwarded request is never this machine's own, and the curator's grants
  decide what the other machine may do. XenoNet's own docs say how to publish
  the service. A tunnel or reverse proxy of your own works the same way.

## Repo layout

```
rust/crates/core/      xenolith-core: paths, pages and frontmatter, index, log, search, read, inbox, locks,
                       request log, secret guard, the curator, Codex runs, Ask's worker, spaces,
                       access, the audit chain, sync
rust/crates/cli/       xenolith: serve (MCP over stdio and HTTP, /health, /status, /ui, /api, /sync),
                       hook, curator, service (Windows), install and uninstall, migrate agent-wiki
                       (journal, rollback, finish; its fixture tests in src/migrate/tests.rs and
                       tests/migrate_agent_wiki.rs), spaces, grants, tokens, audit, sync and held changes
rust/crates/tray/      xenolith-tray: the tray or menu bar icon, its menu, and the window
protocol/PROTOCOL.md   rules every session follows (seeded into a new folder; you own its copy)
protocol/POINTER.md    short global-instruction block
FORMAT.md              the folder's format (version 1), for programs that read it
ui/                    the Xenolith window (React): index.html, src/main.jsx, app.css, markdown.js, api.js
assets/icon/           icon sources (SVG)
plugin/                plugin source; __PLACEHOLDERS__ are filled at install time
scripts/               build (the window), icons, package (the payload and the release archives),
                       verify-archive, release-draft, install-local, ui-preview, eval, bench
docs/releases/         each release's notes, which its GitHub release shows
eval/synthetic/        the memory eval's synthetic wiki, questions and curator note streams
eval/local/            your own real-wiki questions (gitignored)
test/                  the suites: e2e (two real MCP clients, both transports), curator (fake model),
                       tray, installer, the window (ui), Ask (the fake model drives the real read-only
                       server), portable (Linux/macOS differences), memory-eval, spaces, tokens,
                       audit, resources, sync, the archive writers, opt-in live and stress tests
```

The suites talk to the programs over MCP, `/api` and their files: `npm test`
runs them against the programs in `rust/target/release` or `rust/target/debug`
(or `XENOLITH_RUST_BIN`). `scripts/check-rust.sh` formats, lints
(`cargo clippy -D warnings`) and tests the Rust programs on the toolchain
`rust/rust-toolchain.toml` pins. Xenolith's own CI
(`.github/workflows/xenolith.yml`) runs all of it on Linux, macOS and Windows
whenever `xenolith/` changes, and `.github/workflows/xenolith-release.yml`
builds the release archives from a `xenolith-v<version>` tag. `npm run bench` measures the program's footprint and
speed: 5 runs, each metric's median and range, every answer checked, exit 1 on
any error. The memory eval is described in
[docs/memory-eval.md](docs/memory-eval.md).

## Design notes

- Locks use `mkdir .locks/<name>.lock`, the same mutex v1.1 used, so old and new
  servers exclude each other during an upgrade. An `owner` file holds the pid,
  the process start time and a token. Every process also holds
  `.locks/alive/<pid>-<start>.alive` open with share mode 0 for its lifetime.
  Windows closes that file the moment the process dies, so any process, under
  any account, can detect a dead owner and break its lock at once. Another
  process breaks a live owner's lock only after 5 minutes.
- Every write goes to a temp file, then fsync, then rename, with LF line endings
  only. Readers never see a partial file. The server acknowledges a note only
  after it is fsync'd.
- The plugin points at a stable runtime outside the apps' plugin caches, with
  absolute paths written at install time. The hook command uses an 8.3 short
  path when the program's path has spaces, so it runs identically under bash,
  cmd and PowerShell.
- The HTTP server is a small purpose-built HTTP/1.1 server: a thread per
  connection (at most 128), whole request bodies read up front with a 4 MiB
  cap, strict parsing (no line folding, no Content-Length with
  Transfer-Encoding, exactly one Host), and one write per response.
- Windows opens files slowly, because each open is scanned. So the server keeps
  the request log open and caches read-only views of pages, log days and notes,
  keyed by their directory listing's size and time. The release build optimizes
  for size, because launching a larger program costs a longer scan.
