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
  the setting inside the signed start so nothing between her signature
  and the run can drop it ([the guide](../guide/sandbox.md)). A start from
  anyone but her (another machine, a delegation, an intent, an actor) that
  says no sandbox, or nothing, is refused by `xenonet-authz`; a start she
  makes herself has a switch, off on her own machine and on for another
  (decision taken for Nova), and the default allowed domains are none
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

## Next

Each waits until scenario 1 has run for real.

- **Several accounts per app.** Several folders per app, each with its own
  sign-in and timeline; the identity rule gains the account; a start picks
  the folder that holds the right account or refuses.
- **More apps on the map**, through their own documented interfaces.
- **Release watch.** The app registry records which version each reader
  was last checked against, and `xenonet doctor` compares it with what is
  installed.

## Open

- Who sees a run's permission prompt: only the machine that asked for the
  run (today), or every console of the person's own machines.
- Whether a session that takes a message from a guest may still read other
  folders. Sandboxed starts are the mitigation, for the runs XenoNet starts.
- A message already stored is handed over after its project stops being
  shared; closing that needs the message's network in the store.
