# Xenolith: notes for coding agents

Xenolith is shared context for every app and agent on a machine: an MCP server over a folder of
plain Markdown, a curator that files notes into pages, sync between machines, a tray app with a
window, and an installer. It stands alone and runs fully on its own. From XenoNet it may take
only the libraries `xenonet-id`, `xenonet-log`, `xenonet-authz` and `xenonet-net` (with the base
crates they bring: model, os, text, and keys through net), as path dependencies into `../crates`, never the app: not the
service, the runtime, the surfaces or the adapters (`scripts/rust-layers.mjs` fails on one). Other
programs (XenoNet among them) read its folder through `FORMAT.md`. README.md describes what
it does; this file is what an agent changing the code needs. While Xenolith lives in XenoNet's
repository, that repository's AGENTS.md (one folder up) applies too: its commit conventions, and
its rules for working beside other agents.

## Where things are

- `rust/`: the programs that ship, in their own workspace and `Cargo.lock`. `crates/core`
  (xenolith-core: the folder, curator, held changes, search, spaces and their chains, the
  upgrade from 1.x, grants and access, on xenonet-id, -log and -authz), `crates/mcp`
  (xenolith-mcp: the MCP server's tools, resources and prompts), `crates/cli` (`xenolith`: the
  stdio server, the session hook, the commands and the installer, on the core alone, since the hook starts it for every conversation), `crates/service`
  (`xenolith-service`: the HTTP service, the window API, the curator and the Windows service),
  `crates/net` (xenolith-net: Xenolith between machines on xenonet-net, `xenolith/sync/2` and
  `xenolith/join/1` on its own endpoint, and sync/2 over XenoNet's forward in shared mode
  (`shared.rs`), so iroh and tokio; only the service takes it; what sync keeps in
  `<state>/sync/` and what its entries do to the folder are the core's `sync.rs`, what it holds of
  XenoNet (the grant, the chains, binding a peer to its machine) the core's `xenonet.rs`) and `crates/tray` (`xenolith-tray`). Nothing but xenolith-net and the
  service may reach tokio, reqwest, rustls or iroh (`scripts/light.mjs`).
- `ui/`: the window (React), built by `npm run build` into `dist/runtime/ui`.
- `test/*.mjs`: the behaviour suites, run against the Rust programs. `eval/` and `scripts/eval.mjs`:
  the memory eval. `protocol/PROTOCOL.md`: what every connected app is told to do. `FORMAT.md`:
  the folder's format, for programs that read it (XenoNet among them).
- `scripts/`: the window's build, the icons, the release payload and archives (`package.mjs`,
  `archive.mjs`, `verify-archive.mjs`, `release-draft.mjs`), `install-local`, `ui:preview`, the
  eval and the bench. `scripts/lib.mjs` and `scripts/paths.mjs` hold what the
  scripts and tests need of an installed Xenolith.

## Build and check

- `npm run build`: the window, and a check that `package.json` and `rust/Cargo.toml` agree on the
  version.
- `scripts/check-rust.sh` (Linux, WSL or macOS): rustfmt, clippy for this OS and Windows (on a
  Mac, macOS), and the Rust unit tests. Run it before pushing Rust changes. On Windows without
  cargo, run it in WSL. The Windows lint needs MinGW-w64's gcc: XenoNet's libraries build C.
- `npm test` (or `node --test test/<suite>.mjs`): the suites against the Rust programs in
  `rust/target/release`, else `rust/target/debug` (or `XENOLITH_RUST_BIN`, with `xenolith-service`
  beside it or at `XENOLITH_SERVICE_BIN`). Build them first: `cargo build --release --workspace`
  in `rust/` (the tray and installer suites need `xenolith-tray` too).
- To see a change end to end, run an isolated instance: `npm run ui:preview` (a snapshot copy of
  your shared context folder on port 47859), or `xenolith-service serve --http --port 0` with
  `XENOLITH_HOME` and `XENOLITH_DIR` set to temp folders.
- `rust/rust-toolchain.toml` pins the Rust that builds everything, here and in CI. `cargo build`
  in `rust/` builds everything but the tray; `--workspace` adds the tray, which on Linux needs
  `libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev`.
- CI: `.github/workflows/xenolith.yml` runs rustfmt, clippy, the Rust tests and `npm test` on
  Linux, macOS and Windows whenever `xenolith/`, the seven library crates, the root's `Cargo.toml`,
  `Cargo.lock` or `deny.toml` change, and checks the lockfiles and cargo deny once.
- The two lockfiles: `node scripts/xenolith-lock.mjs` (from the repository's root; `npm run
  lint:rust` runs it too) fails when a package XenoNet's libraries reach differs between
  `Cargo.lock` and `rust/Cargo.lock` in version, source or checksum, or when Xenolith turns on a
  feature of it the root doesn't. `--fix` moves `rust/Cargo.lock` to the root's versions. Bump a
  shared dependency in the root first, then `--fix` here.
- `cargo deny --manifest-path xenolith/rust/Cargo.toml --config deny.toml --locked check` (from the
  root) holds Xenolith's lock to the root's `deny.toml`: one list of exceptions for both.

## Releases

A release is a tag `xenolith-v<version>` and four archives, built by
`.github/workflows/xenolith-release.yml`. Each archive holds one folder, `xenolith-<version>-<target>/`
(targets darwin-arm64, darwin-x64, linux-x64, win32-x64), that `xenolith install` installs from.

1. Set the version in `package.json` and `rust/Cargo.toml`, then run `npm install` (the lock file)
   and `cargo build` in `rust/` (Cargo.lock). Behaviour changes raise the minor version.
2. Write `docs/releases/<version>.md`: what changed for a person, what they must do, and how to
   install from the archive on each OS. Review it with XenoNet's unslop guidance.
3. `node scripts/release-draft.mjs --check` checks the versions and the notes.
   `node scripts/package.mjs --target <this machine> --verify` builds this machine's archive into
   `dist/release` and installs it into a fake home (`scripts/verify-archive.mjs`).
4. Push the tag (`git tag -a xenolith-v1.9.0 -m "Xenolith 1.9.0"`). The workflow runs the checks,
   builds and verifies the archives, and makes a draft release "Xenolith <version>" with the
   archives, `SHA256SUMS` and the notes. It never publishes. Run by hand, it uploads the archives
   as workflow artifacts and makes no release.
5. Download and try the archives, then publish the draft with **Set as the latest release**
   unticked: XenoNet's updates read this repository's latest release. Never replace a published
   release's files; ship a new version instead.

## Rules and what enforces them

| Rule | Enforced by |
| --- | --- |
| Public commits use the repository identity; private paths and data stay out of tracked files | `scripts/check-public.mjs`; see `SECURITY.md` |
| Lint every OS target before pushing Rust code | `scripts/check-rust.sh` (local) |
| From XenoNet, only its libraries (xenonet-id, -log, -authz, -net, the base crates and -keys, which -net brings), never the app; nothing of XenoNet's depends on Xenolith | `scripts/rust-layers.mjs` (the root's `npm run lint:rust`) |
| What XenoNet's libraries reach has the same versions, sources and checksums in both lockfiles, and no feature only Xenolith turns on | `scripts/xenolith-lock.mjs` (`npm run lint:rust`, Xenolith's CI) |
| `xenolith` (the hook) and the tray reach no tokio, reqwest, rustls, hyper, iroh or quinn | `scripts/light.mjs` (`check-rust.sh`, Xenolith's CI) |
| `package.json` and `rust/Cargo.toml` carry the same version | `scripts/build.mjs` fails |
| A release's tag, `package.json`, `package-lock.json` and `rust/Cargo.toml` name one version, and its notes exist | `scripts/release-draft.mjs --check`, first in the release workflow |
| A release archive carries no path from the build machine | `checkNoLocalPaths` in `scripts/package.mjs`, after the remapped build |
| A change to `protocol/PROTOCOL.md` records the old version's hash | `wiki::tests::an_edited_protocol_keeps_the_old_hash_for_upgrades` |
| A change to a file `FORMAT.md` describes updates it, and raises `wiki::FORMAT_VERSION` when an older reader would misread the file | `wiki::tests::the_root_says_whose_it_is_and_its_format_and_a_space_does_not` (the marker); judgment |
| Regexes match ASCII digits (`[0-9]`, never `\d`) | `wiki::tests::no_regex_in_the_crates_uses_unicode_digit_classes` |
| Tools are defined once; the `source` enum is `inbox::SOURCES` | `mcp::tool_tests::one_tool_list_with_the_direct_texts_and_the_core_sources` |
| Who may use which space lives in the config folder (access.json), never in the shared context folder; deny by default | `access::policy_file`, `access::tests` |
| Grants, tokens, refusals and writes go in the audit chain, with hashes and revocation ids, never text or a token | `audit::tests`, `test/audit.mjs` |
| Every decision on a space is xenonet-authz's `decide`; a token is a Biscuit grant under this Xenolith's token root, which only the running service holds and mints with; access.json keeps a token's revocation id and row, never the token | `grants::tests`, `access::tests`, `test/tokens.mjs` |
| A request is local only straight from the loopback listener with no `Forwarded`, `X-Forwarded-*` or `Via`; `/status`, `/ui`, `/api` are local only | `http::is_local`, `test/tokens.mjs` |
| Deciding held changes, Undo, Revert, the approvals setting, Forget and notes as the person need the owner key; the CLI goes through the service for them | `uiapi::handle`'s routing, `owner_client`, `test/curator.mjs` |
| A note's source is stamped from who sent it (`by`); only the owner's notes may be `user`, and, from another machine, only a note from a Xenolith bound to a machine whose device is an active device of the space's person owner (stamped `person@<machine>`) | `inbox::By`, `inbox::trusted_source`, `sync::take`, `xenonet::speaks_for`, `inbox::tests`, `sync::tests` |
| Reads return content only: `pages/`, `log/`, `inbox/` and the protocol's top-level files | `wiki::readable_rel`, `wiki::tests::reads_return_content_only` |
| A token reaches `machine:local` only through a grant and scopes that name it exactly | `access::Scope::covers`, `access::tests` |
| Files only the owner may read (access.json, the token roots' certificates, the owner key, the keys) are written with `wiki::write_private` (or, for a key, `xenonet_os::private_file::write_new`): mode 0600 or a protected DACL from the moment they exist, with the Windows service's account as a reader where it must read them | `xenonet_os::private_file::tests`, `keys::tests`, the mode and ACL checks in `test/tokens.mjs` |
| The context signing key, the transport key and the standalone owner key are separate files in `<state>/keys`; one that can't be read is an error, never replaced by a new key | `keys::Keys::load_or_create`, `keys::tests`, `test/tokens.mjs` |
| The standalone owner key signs only for a request with the owner key, or a move to the person drafted for XenoNet's window (`OwnerSigner::to_person`), which the chain takes only beside the person's device and presence; a space it owns is weaker, shares into no network, and moves to a person only, never to a key | `keys::OwnerSigner::signer`; xenonet-log's space policy (`xenonet_log::space`), checked with this Xenolith's own keys in `keys::tests` |
| Each space's chain lives in the state folder; a space folder's `xenolith-space.json` is information only, signed by the context key, and decides nothing; a space followed from another machine is never founded here | `chains::reconcile`, `chains::tests`, `test/spaces.mjs` |
| Between machines a space's members are its chain's key subjects, each admitted only from the endpoint id its share names, and a member admits its curator by the chain or its follow record and the chain's other members by their shares: any other endpoint is refused before anything is read, and a hello whose context key isn't the one bound to its endpoint, or that was signed for another connection, ends it | `xenolith_net::hello`, `engine::Side::hello_ext_in`, `crates/net/tests/sync.rs`, `test/sync.mjs` |
| A member joins only by join/1: an invite (offer hash, inviter endpoint, ticket) and a separate 130-bit code, read from a prompt or stdin; the joiner proves first, a wrong code gets no inviter proof, five retire the invite, an invite works once for ten minutes, and the joiner follows nothing until the chains it was sent check against the offer and share the space with its own key at its own endpoint | `xenolith_net::join`, `crates/net/tests/join.rs`, `test/join.mjs` |
| A member holds the curator's entries it took (`<state>/sync/held/`) and serves them to another member only through xenonet-net's `holder::serve_in`, by its own copy of the chain: to a member at read or above, the curator's feed alone (never a note), at the per-peer limits, and only once it took the requester's chain when the hellos' chain digests differ; a requester asks one holder at a time, ten seconds each | `engine::SpaceHolds`, `engine::Worker::want`, `crates/net/tests/holders.rs` |
| Notes and copy edits go to the curator alone; a member takes only the curator's pages, removals, log days and forgets, of the epoch its chain is at; a peer's note is `agent` (the one exception is the row on `user` notes above); a text travels beside its entry, named by hash, so a forget drops it and the entry stays | `engine::Worker::envelope`, `sync::take`, `sync::tests`, `test/sync.mjs` |
| Shared mode (over XenoNet's forward): the grant goes only to a socket that is this account's alone, answered by a process of this account, and comes back only over it (`collect`, proved with the context key's signature over XenoNet's challenge, then kept with `write_grant`), never through a command's output or a log; XenoNet's chains are verified from genesis here, and its word counts for a connection's remote endpoint alone; a peer is bound to its machine only by its hello's grant, rooted at the machine key the rosters bind to that endpoint and naming the hello's context key (`key(id)`); a bound peer is in a space only by a share into a network both are in, never by a share to a key (the own endpoint's); a member that holds nothing of a space takes its chain only from its curator, when the space's person owner owns a network both are in; a space shared into one network is never told of to a machine in another | `shared::connect_checked`, `xenonet::Known::verify`, `xenonet::bind_hello`, `engine::Worker::network_role`, `engine::Worker::take_offered`, the root's `crates/xenonet-service/tests/xenolith.rs` |
| The copy of the grant a hello carries is narrowed so no door takes it (`check if false`), for an hour | `xenonet::narrow_for_hello`, `grants::tests::the_copy_of_a_grant_a_hello_carries_is_refused_at_every_door` |
| A space moves from the owner key to the person, and shares into a network, only by an entry the curator drafts (asked over the forward session by XenoNet's window, or `spaces draft`), a device of the person signs with their presence, and the curator then sequences; XenoNet signs a draft only once it reads back as exactly the act the person asked for; a network XenoNet says a project isn't shared into any more holds that project's space back at once (`chains::narrowed`): the owner key alone does neither | `chains::draft`, `chains::sequence`, xenonet-log's space policy, xenonet-net's `space_sign` and its tests, `chains::tests` (narrowing); the root's `crates/xenonet-service/tests/xenolith.rs` |
| Sync starts only once the upgrade has run, opens its endpoint only once a space is shared or followed or an invite or a join needs it, answers join/1 only while an invite is open, and uses no relay and no mDNS unless config.json's `sync` names them | `http::start_sync`, `sync::Settings` |
| The endpoint's thread never waits on a file: what reads or writes one is the worker thread's, through a channel | `xenolith_net::engine`; judgment |
| On a folder 1.x left, 2.0 founds, marks, moves and binds nothing until `xenolith upgrade --run`, records each change in `<state>/upgrade.json` before making it, and `--undo` takes back exactly those; it never deletes a 1.x file (`xenolith doctor` names them). A program that may make the folder first opens it with `upgrade::open_folder`, never `wiki::ensure_wiki` | `upgrade::tests`, the upgrade tests in `test/spaces.mjs` |
| Files in the shared context folder are untrusted input (held changes, journals, settings, space markers) | `held::parse_file`, `curator::journal_problem`, `settings::approvals_and_problem`, `chains::reconcile`, and their tests |
| The approval mode is read once per batch, under the commit's lock | `Journal.auto`, `held::apply_auto_locked`; the auto-mode tests in `test/curator.mjs` |
| The installer leaves an app's config alone while that app runs | Claude desktop: `install::desktop_running` and its sandbox test. Codex and Claude Code: judgment |
| Tests and verification never touch the installed service (port 47850) or the real shared context folder | The verify skill's isolated instance; judgment |
| A Rust test that takes a lock deletes its folder with `lock::remove_test_dir`: on Windows a lock holder keeps a file open in that folder for its whole life | The Windows CI job |
| Commit messages are Conventional Commits, one per logical change | Judgment; XenoNet's AGENTS.md |
| Programs keep files in the platform's directories (XDG, AppData, Library), never a new `~/.something` | `xenolith_core::paths`; judgment |

When a mistake repeats, fix it at the highest level that works: remove the way to make it, then a
type, then a check whose error names the fix, then a test. Add the rule here last, with what
enforces it.
