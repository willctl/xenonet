# Always-on XenoNet

A design for running XenoNet's service all the time, with a small tray app
of its own, and the console as a separate app that connects to it. It
covers macOS first, then Linux and Windows. Nothing here is built yet.

**In short:**

- The service starts at login, under the operating system's own
  supervisor (launchd, systemd, Task Scheduler), and comes back if it
  crashes. Closing or updating the console never stops it.
- A small native tray app starts with it. It shows whether XenoNet is
  running, what needs you and the gateway's state, and it opens the
  console.
- The console (Desktop) is only a window onto the service. It no longer
  starts the service or owns the tray.
- Service, Tools, tray and console update separately. They keep agreeing
  through the versioned APIs and schemas in `core/schema` that they
  already share.
- A host is a machine with the service and Tools and nothing else.
  Another machine's console reaches it as it does today, over SSH.

## Why

Coordination shouldn't depend on a window being open. Sessions message
each other, claim files and hear about collisions through the service.
Actors such as Grok Bot reach this machine through the service's gateway.
When nothing starts the service, all of that stops after a restart until
someone opens the console. When the tray lives in the console, closing
the console hides the alerts too.

## What 0.1.3 does

The split into parts already exists (see `docs/guide/components.md`):

- **Desktop, Service and Tools** have separate versions and downloads.
  They negotiate interface versions (service API 1, console API 1) and
  declare data schema 1.
- **The service** is a plain `node server/index.js` process.
  `ensureService` in `core/components/runtime.js` starts it detached, and
  Desktop calls it at startup (`electron/main.js`). Nothing registers it
  with the operating system, so nothing starts it at login or restarts it
  after a crash. The guide says "This release does not register an operating-system
  service or a separate login task."
- **Starting at login** is Desktop's own setting (`electron/login.js`): the
  app is a login item on macOS and a Run entry on Windows. It starts the
  service as a side effect of opening the console.
- **The tray** belongs to Desktop. With Desktop closed, there is no tray.
- **Updates.** Desktop updates itself (on Windows; unsigned Mac builds
  only check). Service and Tools update through Desktop's Updates panel
  or `xenonet update`. The Mac disk image replaces only Desktop.
- **Linux.** The VM lab already runs the service as a systemd user unit
  (`scripts/lab/guest.sh`), but `xenonet setup` doesn't.

## The parts

```
 login / boot
      │
      ▼
 OS supervisor ──starts, restarts──▶ Service (node, 127.0.0.1)
      │                               ▲   ▲   ▲   ▲
      └──starts──▶ Tray app ──────────┘   │   │   │  status, alerts
                                          │   │   │
 Console (Desktop, on demand) ────────────┘   │   │  console API
 Tools: hooks, MCP servers, xenonet ──────────┘   │  service API
 Other machines' consoles (ssh relay) ────────────┘  host relay
```

| Part | Runs | Starts | Updates |
| --- | --- | --- | --- |
| Service | always, as your account | the OS supervisor, at login | itself, when it can restart safely |
| Tools | per command, hook and MCP server | the caller | itself; running processes keep their version |
| Tray | always, on a desktop | the OS, at login, with the service | with the service package, or on its own |
| Console | while you look | you, or the tray | itself |

## Starting at login

Registering is the owner's action, as connecting Claude Code and Codex
is. `xenonet service install` and the console's switch refuse from under
an agent.

### macOS

Two cases:

- **With the tray app** (the normal desktop install). The tray app is a
  small signed bundle. It registers itself with
  `SMAppService.mainApp` so it opens at login, and registers the service
  with `SMAppService.agent(plistName:)`, from a launch agent property list
  in its own `Contents/Library/LaunchAgents`. Its `BundleProgram` is a
  tiny launcher inside the bundle that runs the stable launcher in
  `<state>/runtime/launcher/` with `service run`. The agent gets
  `KeepAlive` (restart on crash, not after a clean stop) and `RunAtLoad`.
  System Settings › General › Login Items shows both under the bundle's
  name, and the person can turn them off there. `status` reports
  `requiresApproval` when they did. SMAppService needs macOS 13, which is
  already XenoNet's minimum.
- **Without a desktop** (a Mac used as a host). `xenonet service install`
  writes `~/Library/LaunchAgents/dev.xenonet.service.plist` with the same
  keys and loads it with `launchctl bootstrap gui/<uid>`.
  `xenonet service uninstall` reverses it with `launchctl bootout`. macOS
  13 and later still list it under Login Items, named after the signer of
  the executable it runs.

Per-user launch agents start at login, not at boot. That's right for
XenoNet, which reads the person's own files. A Mac that should serve as a
host before anyone signs in would need automatic login; a launch daemon
would run as root and is out of scope.

### Linux

`xenonet service install` writes a systemd user unit,
`~/.config/systemd/user/xenonet.service`, with `ExecStart` running the
stable launcher's `service run`, `Restart=on-failure`, and
`WantedBy=default.target`, then enables and starts it. That's what the VM
lab does today. To start at boot without anyone signing in, which a host
needs, the person runs `loginctl enable-linger` once. That may need an
administrator, so XenoNet prints the command rather than running it. The
doctor checks both.

### Windows

The console's installer already writes a Run entry for Desktop. For the
service, the options are a Run entry for the stable launcher, which starts
at sign-in but never restarts a crashed service, or a scheduled task
triggered at logon with restart on failure, which is closer to launchd and
systemd. Whether a standard user can create a logon task without an
administrator needs checking on Windows 11 before choosing.

## The service under a supervisor

- **`service run`.** Supervisors want a process that stays in the
  foreground. `xenonet service run` runs the server in the foreground and
  logs to `<state>/logs`. `service start` stays for installs without a
  supervisor and keeps detaching.
- **One service.** The service already refuses a second copy on its port.
  With a supervisor in charge, `ensureService` and Desktop stop spawning
  their own. They ask the supervisor to start it (`launchctl kickstart`,
  `systemctl --user start`) and wait for the port.
- **Stopping.** `xenonet service stop` stops it until the next login, and
  the supervisor doesn't restart a clean stop. `xenonet service disable`
  (or the tray's Quit XenoNet) turns off starting at login too. Stop keeps
  refusing while terminals run, unless `--force`.
- **Crash loops.** launchd and systemd both throttle restarts. The service
  also writes the reason for each exit to its log, so the doctor and the
  tray can say why it keeps stopping.

## The tray app

**What it does:**

- Shows one icon whose state says running, needs you, gateway online, or
  stopped.
- Its menu lists what needs you (questions, collisions, stuck sessions)
  with the same words as the console's alerts, then Open console, the
  gateway's state with Turn off, Pause notifications, Start at login, and
  Quit XenoNet.
- Posts system notifications. A signed app bundle can do that, and the
  bare node service can't on macOS.
- Opens the console, or the browser console at `http://127.0.0.1:47840`
  when Desktop isn't installed.

**How it talks to the service.** It's another client of the service's
local API, with a credential of its own, like Tools. It uses a small,
versioned tray API, made of a status snapshot and an event stream of alerts. Both
are defined in `core/schema` beside the console API, so the service, the
console and the tray all read the same definitions. The native app gets
its types from those schemas at build time, rather than a second
hand-written copy.

**What it's built with.** macOS first, in Swift with AppKit's
`NSStatusItem` (or SwiftUI's `MenuBarExtra`), as an `LSUIElement` app, with no
Dock icon and a few megabytes of memory. Windows and Linux follow later, each
with a small native helper. Until then, Desktop keeps its tray on those
systems.

**Where it lives.** On macOS it ships inside the macOS Service package,
since a tray without a service is no use, and is installed to
`~/Applications/XenoNet Tray.app`, or `/Applications` if the person
prefers. It updates when the service does. A tray-only update can come
later if it's ever needed.

## Updates

Each part keeps its own version and updates itself. They stay compatible
because each connection negotiates an interface version and checks
capabilities, which XenoNet already does.

- **Service.** It checks for updates on its own, downloads and verifies
  them as `xenonet update service` does now, and installs when nothing
  would be lost, which means when no terminals are running, or else at the next login. It installs
  by exiting with a code that tells the supervisor to start it again
  through the stable launcher, which picks the new version. If the new
  version fails to start, the launcher goes back to the previous one, as
  `recoverServiceUpdate` does today. The tray says when an update waits on
  open terminals.
- **Tools.** No change. New commands and MCP connections use the new
  version, and running ones finish on theirs.
- **Tray.** Updated with the service package. A tray that's too old for
  the service says so and offers the update.
- **Console.** Desktop updates itself. The Mac disk image installs Desktop
  and the tray, and asks the service to update the rest, so one download
  is a complete update.

## Shared schemas

`core/schema` stays the one place the data and API shapes are defined
(see `docs/design/schemas.md` and ADR 0001). This design adds the tray API
there and doesn't change data schema 1. The service owns the data. The
console, Tools, the tray and other hosts read it through versioned APIs
and never open the database themselves.

## Hosts without a console

A host installs the Service and Tools archives, runs `xenonet setup`, and
`xenonet service install`. From then on its service runs from login (or
boot, with lingering) and answers `xenonet relay` over SSH as it does
today. No tray, no console. The console on another machine adds it with
`xenonet hosts add`. That stays the person's action, as it is today.

## Security

- **More hours online.** An always-on service keeps the actor gateway up
  whenever the machine is, if the person turned it on. The tray shows the
  gateway's state and turns it off in one click. The gateway's tokens and
  checks don't change.
- **Same local surface.** The service still listens only on 127.0.0.1,
  with the same Host and Origin checks and per-client credentials. The
  tray gets its own credential, scoped to the tray API.
- **Owner's actions stay the owner's.** Installing, enabling and
  disabling the service refuse from under an agent, and so do the tray's
  gateway and actor controls.

## Moving from 0.1.x

On the first launch of a console with this change, XenoNet asks once
whether to run in the background from login, and explains what that
means. Yes registers the service and the tray, and moves "Open at Login"
from the console to the tray. No keeps today's behavior. The console's
setting can change it later, and so can `xenonet service install` or
`uninstall`.

## Signing and identity

macOS lists login items under the name of the signer of the bundle or
executable they run. XenoNet's Mac builds are signed ad hoc today, with no
team, so Gatekeeper rejects them on first open and Login Items can't show
a publisher. A Developer ID certificate fixes both, but a personal
certificate publishes its owner's legal name in every signature. XenoNet
should be signed only under an organization's name. Until then:

- The tray app is signed ad hoc like the console. It needs the same
  first-open approval, and Login Items lists it by the bundle's name.
- The headless launch agent runs the bundled `node`, so Login Items may
  list it as `node`.

The release workflow already supports Developer ID signing and
notarization (`MAC_SIGN`), so switching is configuration once the
organization's certificate exists.

## Open questions

- Swift or SwiftUI for the tray, given macOS 13 as the minimum.
- On Windows, whether a standard user can create a logon task with
  restart on failure, or whether the service needs a small watchdog in
  the launcher.
- Whether the tray should also ship for Linux desktops in the first
  version, using a StatusNotifierItem, or wait.
- Whether to keep `service start` (detached) once every platform has a
  supervisor.

## Build plan

1. **Service under a supervisor, macOS and Linux.** `service run`,
   `service install`, `uninstall` and `disable`. The launch agent and
   systemd unit, `ensureService` asking the supervisor, doctor checks,
   tests with fake supervisors. Desktop stops spawning the service when a
   supervisor owns it.
2. **Tray API.** The schema in `core/schema`, the status and event
   endpoints, the tray's credential, and tests.
3. **macOS tray app.** The Swift app, `SMAppService` registration of
   itself and the service agent, packaging in the macOS Service archive,
   and the release workflow.
4. **Updates.** The service's own update check and install at a safe
   moment, recovery through the launcher, and the disk image completing
   the update.
5. **The move from 0.1.x.** The first-run question, moving Open at Login,
   and the guide's components page.
6. **Windows.** After the open question is settled.

## Sources

- Apple, Service Management: [`SMAppService`](https://developer.apple.com/documentation/servicemanagement/smappservice),
  [`agent(plistName:)`](https://developer.apple.com/documentation/servicemanagement/smappservice/agent%28plistname%3A%29),
  [`register()`](https://developer.apple.com/documentation/servicemanagement/smappservice/register%28%29),
  [Updating helper executables from earlier versions of macOS](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos).
- XenoNet: `docs/guide/components.md`, `docs/design/contracts.md`,
  `docs/design/hosts.md`, `docs/design/schemas.md`, `scripts/lab/guest.sh`.
