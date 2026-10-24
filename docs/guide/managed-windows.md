# Managed Windows installation

Use a fixed installation candidate when application control requires
approval. Record its source commit, workflow run, component versions and
SHA-256 hashes. An approval applies to those files. A rebuilt or updated
file needs a matching approval or a publisher rule chosen by the
administrator.

## What to install

Install the Windows bundle as the person who uses Codex and Claude Desktop.
It contains Desktop, Service and Tools. Service and Tools use the Rust
`xenonet.exe` binary and do not require a separate Node installation.
Desktop uses Electron and includes its own runtime and native libraries.
Approve the runtime files as well as the installer.

The service runs as the signed-in user. Its optional background task is
named `XenoNet Service`. It does not need to run as SYSTEM. The owner turns
background startup on in XenoNet or from their own terminal. XenoNet
refuses that change from an agent.

For Hermes in WSL, stage the Linux x64 Service and Tools packages in the
distribution where Hermes runs. Set `HERMES_HOME` in the collector's own
environment to the existing Hermes data directory. XenoNet's observer does
not need Hermes's OpenRouter key. Run the collector where it can see the
Hermes process and metadata. If Hermes itself runs inside a container,
validate that visibility there separately. A collector in another process
namespace may not see it.

Each collector reads its own environment. The owner pairs them using
XenoNet's network controls. On Windows, adding a machine and reading remote
transcripts require XenoNet's window. A browser can show the local console
but does not replace those controls. See [Network](network.md).

## What an administrator reviews

The approval packet should contain the exact files, hashes, signature
results and source provenance. Include the installer, installed Desktop
executables and libraries, the component binaries and the stable Tools
launcher. Record the file inventory from the unpacked candidate, then
compare it with the installed files during the pilot.

Default Windows locations are:

| Location | Use |
| --- | --- |
| `%LOCALAPPDATA%\Programs\XenoNet` | Per-user Desktop installation |
| `%LOCALAPPDATA%\XenoNet` | State, index, logs, component packages and runtime |
| `%LOCALAPPDATA%\XenoNet\runtime\launcher\xenonet.exe` | Stable command, hooks and MCP launcher |
| `%LOCALAPPDATA%\XenoNet\bin` | Command entry added to the user's PATH |
| `%APPDATA%\XenoNet` | Electron preferences |
| `%USERPROFILE%\.codex` | Codex session data and reviewed connection configuration |
| `%USERPROFILE%\.claude` | Claude Code transcripts and reviewed connection configuration |
| `%APPDATA%\Claude`, or Claude's MSIX redirected LocalCache folder | Claude Desktop coding-session metadata |

These are defaults. Environment overrides and app packaging can move them.
Inventory actual locations without collecting conversation text or keys.
`Connect your agents to XenoNet` writes hooks and MCP entries to the apps'
configuration after review. Collection also writes XenoNet's own state and
search index. Describe those writes in the approval request.

The local console uses `127.0.0.1:47840`. The service also uses authenticated
loopback connections whose port is recorded in its runtime descriptor.
Network pairing introduces a separate gateway address. Record that exact
address and port during the pilot. Do not open a broad inbound rule or
enable a public tunnel as part of basic installation.

XenoNet has command, terminal, messaging, provider and network features.
XenoNet does not enforce an observation-only policy. Review those
capabilities even when the pilot uses only collection. Keep optional cloud
providers and public tunnels unconfigured for the initial test. An
existing installation may already hold provider settings, so inspect the
settings in XenoNet before enabling collection. Never export credential
values into the approval packet.

Use a narrow file-hash approval for an unsigned pilot. The administrator
chooses any publisher, path, child-process, storage or network rules.
Signing establishes publisher identity and integrity; it does not grant
approval by itself. Avoid broad interpreter or project-directory rules.
Use the endpoint audit to identify an actual denied helper or file access.

## Pilot sequence

1. Verify the candidate's hashes. Record installed versions, startup entries
   and connection configuration. Close any XenoNet terminals before changing
   the service. Keep a private backup of the previous app, XenoNet state and
   the configuration files the connection step will change.
2. Have the administrator approve the candidate's exact runtime files.
   Start with no automatic startup. The bundle accepts
   `/currentuser /NOAUTOSTART /NODESKTOP`; interactive setup lets the owner
   choose the installation and launch steps. See the packet for exact files.
3. Launch Desktop from the owner's session. Inspect Updates and turn off
   automatic checks for the fixed-hash pilot. Confirm the reported Desktop,
   Service and Tools versions before connecting agents.
4. Review the connection changes for Codex and Claude Code. Reconnect the
   apps' MCP clients as needed. Use made-up coding tasks to check discovery,
   activity, waiting states and completion. Existing Claude Desktop coding
   sessions use its local records. Ordinary cloud conversations have
   separate coverage and are not a success criterion for this pilot.
5. Start the Linux collector with the existing Hermes data path and without
   copying its provider key. Use a harmless made-up task to check Hermes's
   activity and observed edit metadata. Preserve the existing Hermes
   credential bridge and runtime configuration.
6. The owner pairs Windows and WSL and checks both collectors on the map.
   Compare each displayed state with a known test action. Then test sign-out,
   reboot and startup, once startup has been explicitly enabled by the owner.
7. Record pass, fail or not tested for every check. Keep screenshots of real
   sessions and endpoint audit exports outside the repository. Packaged
   restart and GPU checks remain separate from hosted software-rendered CI.

## Stop or remove the pilot

Stop new test tasks and close XenoNet terminals first. The owner turns off
background startup with `xenonet service uninstall` and reviews
`xenonet disconnect` to remove the connection entries XenoNet added.
Stop the WSL collector using the mechanism that started it. Remove the
Windows app through Installed apps. Keep its data for diagnosis unless the
owner chooses removal. `/PURGE` deletes saved state and preferences and is
not the default rollback procedure.

Keep the private preflight backup until the pilot is accepted. Reinstalling
an older binary does not prove that it can read state a newer version
wrote. Restore the matching backup only after stopping the candidate and
reviewing the exact restore paths with the owner.

An installation is ready for use only after it works under the intended
endpoint policy and resumes collection after the agreed startup test.
Building the installer and obtaining a temporary learning window do not
establish that result.
