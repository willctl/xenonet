# Daily use

What it takes for one person to run a week of their real agent work on
XenoNet. The person here is Nova: `desk` and `tower` are her machines,
`contractor-vm` is a Linux machine she works on and doesn't own, `atlas` is
the project that machine is for, and `ledger` is any other project. The
names are made up.

## What it's for

1. A session on `desk` hands a render to `tower` and gets the result back.
2. A manager session on `desk` starts work on `contractor-vm` in `atlas`,
   answers its questions from `desk`, and `contractor-vm` can reach
   nothing but `atlas`.
3. Every session runs as the account and git identity its project calls
   for, and a mismatch is caught before a commit or a push.
4. Sessions started from the network run sandboxed.
5. Sessions from the apps she uses show on one map.
6. When a provider ships a new version, XenoNet says whether it still
   reads it.

Scenario 1 needs no new code, only both machines on a current release.

## Built

- **Harness.** The engine test harness is shared
  (`crates/xenonet-service/tests/common/engine.rs`), with
  `two_networks()`: `desk` in `home` with `tower` and in `work` with
  `contractor-vm`, `work` shown only `atlas`. The lab's VMs reach each
  other on Lima's `user-v2` network ([the lab's README](../../scripts/lab/README.md)).
- **An identity guard that warns.** A project's rule names its git email
  and push account; a mismatch shows in Needs you and the inspector, a
  session is told at its start, and an opt-in pre-push hook refuses the
  push ([identities](../guide/identities.md)).
- **A network reaches only what it sees.** The receiving machine decides
  every act from another machine against what the network the frame came
  in is shown, before it checks the sender; what leaves is held to the
  same sharing; a network with no sharing row is shown nothing; the first
  network no longer stands in for every network
  ([authorization](authorization.md), [network](network.md)).
- **A machine she doesn't own.** Nothing new was needed: a start she signs
  at `desk` runs on a machine that holds no identity of hers, in a shared
  project's folder, and its prompts come back to `desk`. From
  `contractor-vm` a start or wake on `desk` outside `atlas`, a transcript
  read and anything in `ledger` are refused (tests in `net_engine.rs`). A
  guest grant in the network's own chain is built only if asking for
  presence on every prompt is too slow in use. Whoever holds root on that
  machine can ignore anything XenoNet does there; what protects her is
  that her own machines decide ([the guide](../guide/network.md#working-on-a-machine-you-dont-own)).
- **Sandboxed starts.** Each harness's own sandbox, driven by XenoNet, with
  the confinement level inside the signed start (`off`, `vendor` or
  `cell`, [confinement](confinement.md)) so nothing between her signature
  and the run can drop it ([the guide](../guide/sandbox.md)). A start from
  anyone but her (another machine, a delegation, an intent, an actor) that
  says `off`, `cell` or nothing is refused by `xenonet-authz`; a start she
  makes herself has a level, `off` on her own machine and `vendor` for
  another (decision taken for Nova), and the default allowed domains are none
  (decision taken). Claude Code gets one settings file per run that
  replaces the repo's own settings and servers; Codex gets
  `--sandbox workspace-write`, approvals never and no network, and refuses
  domains; Grok Build and Hermes have no mapping and refuse. A turn resumed
  or forked for a sandboxed session is sandboxed, and a call that asks to
  leave the sandbox is never answered by a delegation or a grant. Decisions
  taken: a sandbox is never the home folder, the root or what holds home;
  domains from anyone but her are limited to a list she sets (none by
  default); a sandboxed Codex start from anyone but her is refused, since
  its sandbox reads the whole disk. Left: Claude Code's sandbox wraps shell
  commands only, so a fetch of another domain, with domains given, asks.
- **Several accounts per app.** An app is read from its own folder and from
  those she names (`xenonet folders`, yours to change, read when the service
  starts); an adapter proposes candidates and nothing is read from one she
  hasn't named. Each folder has its own lister, watcher, transcripts,
  sign-in file and timeline, so a session in a named folder is linked to its
  account by its folder, and hooks, plan usage, resume, copy and wake take
  the session's own folder. The identity rule gains `--claude`, `--chatgpt`
  and a mode ([identities](../guide/identities.md#several-folders-for-an-app)).
  Decisions taken: the app's own folder is always read, the others only when
  named; a rule's mode is `warn` unless she says `refuse`; a start into a
  project whose rule names an account no folder holds is refused in either
  mode, never run under another; a rule-bound or named-folder run signs in
  with its folder's own sign-in, gets no shared runs token and none of the
  app's API-key variables, so a sandboxed start into such a project is
  refused, and so is every start that is always sandboxed (another machine, a
  delegation, an intent, a session, an actor); a start's placement (folder,
  withheld token, deny arguments) is kept in the store per session and
  carried to its copies, applied to every later turn after the rule is read
  again, and a turn whose record can't be read is refused; refuse mode,
  which needs no account in the rule, passes Claude Code `--disallowedTools`
  for `git commit`, `git push` and `gh`, which joins the deny rules of the
  run's settings. That blocks a command that starts with one of them, each
  part of a command joined with `&&`, `;` or `|` included. It does not block
  `git -C x push`, `git -c k=v commit`, `/usr/bin/git push`, `env git push`,
  `sh -c`, aliases, scripts, `make` or `npm run`, a GitHub MCP server or
  `curl` with a token; the pre-push hook (`xenonet identity hook install`) is
  what stops a push from the repo. A folder's keychain entry is not read for
  plan usage; a session id found in two named folders has no folder and is
  refused; taking a folder away takes XenoNet's hooks out of it. Left: the page has
  no place to name folders yet, and the saved folders take effect only after
  the service starts again.
- **Release watch.** The app registry records, per app, the version its
  readers were last checked against (`checked`: Claude Code 2.1.327 and
  Codex 0.150.0, which the repo's own code and golden world pin, and Grok
  Build 1.0.77, the version installed when it was set because nothing
  pins one). `claude.release`, `codex.release` and `grok.release` in the
  doctor run the app's own `--version` (4 second timeout, no network) and
  compare: a newer version warns with both versions, an older one is a
  note, an app that isn't installed is skipped, and a probe that hangs or
  prints no version is said plainly. The drift warning names the version a
  change was first seen with. Decisions taken: the check is one row per
  app beside its `.cli` row, since the doctor stays read-only and offline;
  only apps with a version probe and a native reader have a checked
  version (Hermes and Claude desktop have neither); prerelease tags are
  ignored in the comparison. Left: asking vendors for their latest
  version, and the weekly routine that moves `checked` after a manual
  check; both are outside the doctor.

- **T3 Code on the map.** T3 Code runs Claude Code through the Agent SDK
  with the person's settings, so XenoNet's hooks and MCP server work in its
  threads with nothing built. The Claude reader reads the transcript's
  `entrypoint`: a session whose entrypoint is `t3-code` has `runsIn:
  "t3-code"` (every other Claude session `null`) and the inspector's origin
  row says T3 Code. A session in a linked git worktree is in the project of
  the main checkout, wherever the worktree is (T3 Code's `~/.t3/worktrees`,
  `git worktree add ../x`, any tool's), when that checkout is a project the
  map has and shares its git folder; its folder is the main checkout's and
  its worktree the worktree's. The demo has one such session in atlas-web
  ([the guide](../guide/agents.md#t3-code)). Seen on a real install (T3 Code
  0.1.11, Claude Code 2.1.288): the hook environment has
  `CLAUDE_CODE_ENTRYPOINT=t3-code` and `T3CODE_THREAD_ID`, kept for starting
  and messaging threads later. Decision taken: the worktree rule is for every
  app and tool, not T3 Code alone. Unverified: Codex under T3 Code (its
  threads failed before writing anything while Codex was signed out), and
  any session while Claude Code is signed in, since the install was signed
  out. T3 Code's threads default to full access with no OS sandbox, so
  confined starts don't cover them. Left: reading T3 Code's SQLite or MCP
  server, starting and messaging threads by the thread id, a release watch
  row for T3 Code, detecting Codex under it.

## Next

- **More apps on the map**, through their own documented interfaces.

## Open

- Who sees a run's permission prompt: only the machine that asked for the
  run (today), or every console of the person's own machines.
- Whether a session that takes a message from a guest may still read other
  folders. Sandboxed starts are the mitigation, for the runs XenoNet starts.
- A message already stored is handed over after its project stops being
  shared; closing that needs the message's network in the store.
- No confined command has been seen blocked in a real run yet: that needs
  Claude Code and Codex signed in on the machine that tests it.
- Cell starts are refused until a cell can carry the app's sign-in and
  bring its session back to the map.
