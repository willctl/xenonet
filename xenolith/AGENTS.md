# Xenolith: notes for coding agents

Xenolith is one shared memory for every app and agent on a machine: an MCP server over a
plain-Markdown wiki, a curator that files notes into pages, a tray app with a window, and an
installer. It is XenoNet's shared context and also works on its own. README.md describes what it
does; this file is what an agent changing the code needs. XenoNet's own AGENTS.md (one folder up)
applies too: its commit conventions, and its rules for working beside other agents.

## Where things are

- `rust/`: the programs that ship. `crates/core` (xenolith-core: wiki, curator, held changes, search,
  spaces and access),
  `crates/cli` (`xenolith`: MCP server, HTTP service, window API, installer), `crates/tray`
  (`xenolith-tray`).
- `ui/`: the window (React), built by `npm run build` into `dist/runtime/ui`.
- `src/`, `service/`, `tray/*.cs`, `scripts/install.mjs`: the earlier Node runtime and C# programs.
  They are retired by R5 (`docs/rust-plan.md`). Do not add features there.
- `test/*.mjs`: the behaviour suites for both implementations. `eval/` and `scripts/eval.mjs`:
  the memory eval. `protocol/PROTOCOL.md`: what every connected app is told to do.

## Build and check

- `npm run build`: the window, and a check that `package.json` and `rust/Cargo.toml` agree on the
  version.
- `scripts/check-rust.sh` (Linux, WSL or macOS): rustfmt, clippy for this OS, Windows and macOS,
  and the Rust unit tests. Run it before pushing Rust changes. On Windows without cargo, run it in
  WSL.
- `XENOLITH_IMPL=rust node --test test/<suite>.mjs`: the suites against the Rust programs in
  `rust/target/release` (or `XENOLITH_RUST_BIN`). A plain `npm test` runs them against the Node
  runtime and skips the Rust-only tests (the trust gate, held changes, forget, the Rust installer
  and tray), so run both.
- To see a change end to end, run an isolated instance: `npm run ui:preview` (a snapshot copy of
  your wiki on port 47859), or `xenolith serve --http --port 0` with `XENOLITH_HOME` and
  `XENOLITH_DIR` set to temp folders.
- Rust 1.85 builds the core and the CLI (`cargo build` in `rust/`). The tray needs Rust 1.90:
  `cargo +1.90 build -p xenolith-tray`, or that toolchain's cargo with `RUSTC` set to its rustc.

## Rules and what enforces them

| Rule | Enforced by |
| --- | --- |
| Public commits use the repository identity; private paths and data stay out of tracked files | `scripts/check-public.mjs`; see `SECURITY.md` |
| Lint every OS target before pushing Rust code | `scripts/check-rust.sh` (local) |
| `package.json` and `rust/Cargo.toml` carry the same version | `scripts/build.mjs` fails |
| A change to `protocol/PROTOCOL.md` records the old version's hash | `wiki::tests::an_edited_protocol_keeps_the_old_hash_for_upgrades` |
| Regexes match ASCII digits (`[0-9]`, never `\d`) | `wiki::tests::no_regex_in_the_crates_uses_unicode_digit_classes` |
| Tools are defined once; the `source` enum is `inbox::SOURCES` | `mcp::tool_tests::one_tool_list_with_the_direct_texts_and_the_core_sources` |
| Who may use which space lives in the config folder (access.json), never in the wiki; deny by default | `access::policy_file`, `access::tests` |
| Grants, tokens, refusals and writes go in the audit chain, with hashes, never text | `audit::tests`, `test/audit.mjs` |
| A request is local only straight from the loopback listener with no `Forwarded`, `X-Forwarded-*` or `Via`; sync always needs a token; `/status`, `/ui`, `/api` are local only | `http::is_local`, `test/tokens.mjs` |
| Deciding held changes, Undo, Revert, the approvals setting, Forget and notes as the person need the owner key; the CLI goes through the service for them | `uiapi::handle`'s routing, `owner_client`, `test/curator.mjs`, `test/sync.mjs` |
| A note's source is stamped from who sent it (`by`); only the owner's notes may be `user` | `inbox::By`, `inbox::trusted_source`, `inbox::tests` |
| Reads return content only: `pages/`, `log/`, `inbox/` and the protocol's top-level files | `wiki::readable_rel`, `wiki::tests::reads_return_content_only` |
| A token reaches `machine:local` only through a grant and scopes that name it exactly | `access::Scope::covers`, `access::tests` |
| Files only the owner may read (access.json, sync.json, peer tokens, the owner key) are written with `private::write` | `private::tests`, the mode checks in `test/tokens.mjs` and `test/sync.mjs` |
| Files in the wiki folder are untrusted input (held changes, journals, settings) | `held::parse_file`, `curator::journal_problem`, `settings::approvals_and_problem`, and their tests |
| The approval mode is read once per batch, under the commit's lock | `Journal.auto`, `held::apply_auto_locked`; the auto-mode tests in `test/curator.mjs` |
| The installer leaves an app's config alone while that app runs | Claude desktop: `install::desktop_running` and its sandbox test. Codex and Claude Code: judgment |
| Tests and verification never touch the installed service (port 47850) or the real wiki | The verify skill's isolated instance; judgment |
| Commit messages are Conventional Commits, one per logical change | Judgment; XenoNet's AGENTS.md |
| Programs keep files in the platform's directories (XDG, AppData, Library), never a new `~/.something` | `xenolith_core::paths`; judgment |

When a mistake repeats, fix it at the highest level that works: remove the way to make it, then a
type, then a check whose error names the fix, then a test. Add the rule here last, with what
enforces it.
