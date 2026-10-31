# Xenolith: notes for coding agents

Xenolith is shared context for every app and agent on a machine: an MCP server over a folder of
plain Markdown, a curator that files notes into pages, sync between machines, a tray app with a
window, and an installer. It stands alone: nothing in it needs XenoNet's code or concepts, and
other programs (XenoNet among them) read its folder through `FORMAT.md`. README.md describes what
it does; this file is what an agent changing the code needs. While Xenolith lives in XenoNet's
repository, that repository's AGENTS.md (one folder up) applies too: its commit conventions, and
its rules for working beside other agents.

## Where things are

- `rust/`: the programs that ship. `crates/core` (xenolith-core: the folder, curator, held changes,
  search, spaces, access and sync),
  `crates/cli` (`xenolith`: MCP server, HTTP service, window API, installer), `crates/tray`
  (`xenolith-tray`).
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
- `scripts/check-rust.sh` (Linux, WSL or macOS): rustfmt, clippy for this OS, Windows and macOS,
  and the Rust unit tests. Run it before pushing Rust changes. On Windows without cargo, run it in
  WSL.
- `npm test` (or `node --test test/<suite>.mjs`): the suites against the Rust programs in
  `rust/target/release`, else `rust/target/debug` (or `XENOLITH_RUST_BIN`). Build them first:
  `cargo build --release --workspace` in `rust/` (the tray and installer suites need
  `xenolith-tray` too).
- To see a change end to end, run an isolated instance: `npm run ui:preview` (a snapshot copy of
  your shared context folder on port 47859), or `xenolith serve --http --port 0` with `XENOLITH_HOME` and
  `XENOLITH_DIR` set to temp folders.
- `rust/rust-toolchain.toml` pins the Rust that builds everything, here and in CI. `cargo build`
  in `rust/` builds the core and the CLI; `--workspace` adds the tray, which on Linux needs
  `libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev`.
- CI: `.github/workflows/xenolith.yml` runs rustfmt, clippy, the Rust tests and `npm test` on
  Linux, macOS and Windows whenever `xenolith/` changes. It needs nothing from XenoNet.

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
| `package.json` and `rust/Cargo.toml` carry the same version | `scripts/build.mjs` fails |
| A release's tag, `package.json`, `package-lock.json` and `rust/Cargo.toml` name one version, and its notes exist | `scripts/release-draft.mjs --check`, first in the release workflow |
| A release archive carries no path from the build machine | `checkNoLocalPaths` in `scripts/package.mjs`, after the remapped build |
| A change to `protocol/PROTOCOL.md` records the old version's hash | `wiki::tests::an_edited_protocol_keeps_the_old_hash_for_upgrades` |
| A change to a file `FORMAT.md` describes updates it, and raises `wiki::FORMAT_VERSION` when an older reader would misread the file | `wiki::tests::the_root_says_whose_it_is_and_its_format_and_a_space_does_not` (the marker); judgment |
| Regexes match ASCII digits (`[0-9]`, never `\d`) | `wiki::tests::no_regex_in_the_crates_uses_unicode_digit_classes` |
| Tools are defined once; the `source` enum is `inbox::SOURCES` | `mcp::tool_tests::one_tool_list_with_the_direct_texts_and_the_core_sources` |
| Who may use which space lives in the config folder (access.json), never in the shared context folder; deny by default | `access::policy_file`, `access::tests` |
| Grants, tokens, refusals and writes go in the audit chain, with hashes, never text | `audit::tests`, `test/audit.mjs` |
| A request is local only straight from the loopback listener with no `Forwarded`, `X-Forwarded-*` or `Via`; sync always needs a token; `/status`, `/ui`, `/api` are local only | `http::is_local`, `test/tokens.mjs` |
| Deciding held changes, Undo, Revert, the approvals setting, Forget and notes as the person need the owner key; the CLI goes through the service for them | `uiapi::handle`'s routing, `owner_client`, `test/curator.mjs`, `test/sync.mjs` |
| A note's source is stamped from who sent it (`by`); only the owner's notes may be `user` | `inbox::By`, `inbox::trusted_source`, `inbox::tests` |
| Reads return content only: `pages/`, `log/`, `inbox/` and the protocol's top-level files | `wiki::readable_rel`, `wiki::tests::reads_return_content_only` |
| A token reaches `machine:local` only through a grant and scopes that name it exactly | `access::Scope::covers`, `access::tests` |
| A pairing code works once, within ten minutes, only with its secret; the token it yields reads and appends only; neither the code nor the token enters the audit chain | `pair::accept`, `test/pair.mjs` |
| Files only the owner may read (access.json, sync.json, peer tokens, the owner key) are written with `private::write` | `private::tests`, the mode checks in `test/tokens.mjs` and `test/sync.mjs` |
| Files in the shared context folder are untrusted input (held changes, journals, settings, sync state) | `held::parse_file`, `curator::journal_problem`, `settings::approvals_and_problem`, sync's `load_manifest` and `FollowState::load`, and their tests |
| The approval mode is read once per batch, under the commit's lock | `Journal.auto`, `held::apply_auto_locked`; the auto-mode tests in `test/curator.mjs` |
| The installer leaves an app's config alone while that app runs | Claude desktop: `install::desktop_running` and its sandbox test. Codex and Claude Code: judgment |
| Tests and verification never touch the installed service (port 47850) or the real shared context folder | The verify skill's isolated instance; judgment |
| A Rust test that takes a lock deletes its folder with `lock::remove_test_dir`: on Windows a lock holder keeps a file open in that folder for its whole life | The Windows CI job |
| Commit messages are Conventional Commits, one per logical change | Judgment; XenoNet's AGENTS.md |
| Programs keep files in the platform's directories (XDG, AppData, Library), never a new `~/.something` | `xenolith_core::paths`; judgment |

When a mistake repeats, fix it at the highest level that works: remove the way to make it, then a
type, then a check whose error names the fix, then a test. Add the rule here last, with what
enforces it.
