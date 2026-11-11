# Always-on XenoNet

How XenoNet's service runs all the time, with a small tray app of its own,
and the console as a separate app that connects to it. The guide is
[Keeping the service running](../guide/always-on.md);
[components.md](../guide/components.md) has the parts and their versions.

**In short:**

- The service can start at login under the operating system's own
  supervisor (launchd on macOS, systemd on Linux, Task Scheduler on
  Windows), and comes back if it crashes. It's off until the person turns
  it on. Closing or updating the console never stops it.
- On macOS a small native tray app, XenoNet Tray, starts with it. It shows
  whether XenoNet is running, what needs you and the gateway's state, and
  it opens the console.
- The console (Desktop) is a window onto the service.
- Service, Tools, tray and console have their own versions, and agree
  through versioned APIs and the schemas in `schemas/`
  ([schemas.md](schemas.md)).
- A machine with the Service and Tools and nothing else is a full member
  of a network ([network.md](network.md)).

## Why

Coordination shouldn't depend on a window being open. Sessions message
each other, claim files and hear about collisions through the service.
Actors reach this machine through the service's gateway, and other
machines through its network link. When nothing starts the service, all
of that stops after a restart until someone opens the console.

## The parts

```
 login
   │
   ▼
 OS supervisor ──starts, restarts──▶ Service (xenonet serve, 127.0.0.1)
   │                                  ▲   ▲   ▲   ▲
   └──starts──▶ Tray app ─────────────┘   │   │   │  tray API
                                          │   │   │
 Console (Desktop, on demand) ────────────┘   │   │  console API
 Tools: hooks, MCP servers, xenonet ──────────┘   │  service API
 Other machines of the network ───────────────────┘  the network
```

| Part | Runs | Starts | Updates |
| --- | --- | --- | --- |
| Service | always, as your account | the OS supervisor, at login | `xenonet update service`, or Desktop's Updates panel |
| Tools | per command, hook and MCP server | the caller | `xenonet update tools`; running processes keep their version |
| Tray (macOS) | always, on a desktop | its own launch agent, at login | a newer tray component from the disk image's bundle |
| Console | while you look | you, or the tray | itself |

## Starting at login

`xenonet service install` registers the service and `uninstall` removes
it. Desktop offers it once, during setup, and Background service (in the
palette, the tray and the macOS menu) has the switch; both run those
commands. Registering is the owner's action: `install` and `uninstall` ask
the running service whether the command comes from you, as the routes
that are yours do, and with no service running the command looks at its
own parent processes the same way. They're refused from under an agent.

The supervisor runs `xenonet service run` from the launcher's copy of the
binary, `<state>/runtime/launcher/xenonet`, which never moves
([contracts.md](contracts.md), "Integration points"), or from the binary
itself when there's no launcher (a build of your own). `service run`
finishes an interrupted update, then runs the selected Service's
`bin/xenonet serve` with the environment Desktop gives it, the login
shell's `PATH` included.

### macOS

`xenonet service install` writes
`~/Library/LaunchAgents/dev.xenonet.service.plist`, with `KeepAlive`
(restart on crash, not after a clean stop) and `RunAtLoad`, and loads it
with `launchctl bootstrap gui/<uid>`; `uninstall` reverses it with
`launchctl bootout`. On Unix `service run` becomes the service, so
launchd's signals reach it. Login Items lists the agent under the signer
of the executable it runs, and turning it off there keeps launchd from
starting it.

Per-user launch agents start at login, not at boot. That's right for
XenoNet, which reads the person's own files. A Mac that should serve
before anyone signs in needs automatic login; a launch daemon would run
as root and is out of scope.

### Linux

`xenonet service install` writes a systemd user unit,
`~/.config/systemd/user/xenonet.service`, with `ExecStart` running
`service run`, `Restart=on-failure` and `WantedBy=default.target`, then
enables and starts it. To start at boot without anyone signing in, the
person runs `loginctl enable-linger` once. That may need an
administrator, so XenoNet prints the command rather than running it.

### Windows

`xenonet service install` writes the task's definition to
`%LOCALAPPDATA%\XenoNet\xenonet-service.xml` (UTF-16, which
`schtasks /xml` needs), registers it as **XenoNet Service** with
`schtasks /create /xml … /f`, and starts it with `schtasks /run`
(`crates/xenonet-components/src/supervisor.rs`, behind the same `Files`
and `Runner` as launchd and systemd, so its tests run a made-up
schtasks).

- The task runs at the account's own logon, named by its SID, as that
  account with an interactive token and least privilege, so no
  administrator is needed. It has no time limit and no battery
  conditions, runs one copy at a time, and Task Scheduler starts it again
  a minute after it fails to start, three times at most.
- Its command is `conhost.exe --headless` with the launcher's
  `xenonet.exe service run` as its arguments, so no console window opens
  at sign-in.
- A task has no environment of its own, so the kept settings follow
  `service run` as `NAME=VALUE` arguments, and `service run` takes only
  the names it keeps.
- Task Scheduler restarts a task only when it fails to start, so on
  Windows `service run` stays the parent: it starts the service, appends
  its output to the service's log, and starts it again 5 seconds after a
  failure, giving up after 10 starts in 5 minutes, as the systemd unit
  does. A clean stop ends it too, until the next sign-in.
- `service run` keeps its own process and the service's in
  `xenonet-service.pids.json` beside the definition, and `status` counts
  them only while `tasklist` still lists them.
- A task you disable in Task Scheduler is reported as `disabled`, and
  `xenonet service start` leaves it off; `install` turns it on again.
- A changed definition ends the running copy and starts the new one,
  after asking the supervised service to stop cleanly. `uninstall` does
  the same, then `schtasks /end` and `/delete`.

A task that runs as you runs only while you're signed in, so a Windows
machine serves only then.

## The service under a supervisor

- **One service.** The service refuses a second copy on its port.
  `service run` waits while another service has its port, instead of
  exiting, so a registration made while Desktop's own service runs takes
  over when that one stops. `install` stops the running service at once
  when the supervisor's copy is waiting and no terminals are open.
- **Starting it.** `xenonet service start` and `xenonet update service`
  ask the supervisor to start a registered service for their port
  (`launchctl kickstart`, `systemctl --user start`, `schtasks /run`)
  instead of starting one beside it. XenoNet Dev starts and stops its own.
- **Stopping.** `xenonet service stop` stops it until the next login, and
  the supervisor doesn't restart a clean stop. Stop is refused while
  terminals run, unless `--force`. `xenonet service uninstall` stops the
  supervised copy and removes the registration, under the same rule.
  Desktop's uninstallers run it.
- **Crash loops.** launchd and systemd throttle restarts, and `service
  run` does on Windows. The service writes the reason for each exit to its
  log.
- **The doctor.** Four checks under Background service (`service.*`, in
  `crates/xenonet-doctor/src/login.rs`) read the registration as the
  components crate's `supervisor::review` sees it: whether it's there;
  whether its file is what `install` would write now, and whether the
  program it runs is there; whether the supervisor has it loaded and
  running; and on Linux, lingering, a warning only where the account has
  no graphical session.

## The tray app

XenoNet Tray (`tray/macos`, Swift) is an `LSUIElement` app with no Dock
icon. Its menu lists what needs you, with the same words as the console's
alerts, the gateway's state and this machine's link to its network, Open
console, and Quit the tray. It posts one system notification per item
that starts waiting, and withdraws it when the item stops. It opens
Desktop at a session or actor, or the browser console when Desktop isn't
connected.

- **Its API.** `GET /api/tray/v1/status`, the `GET /api/tray/v1/events`
  stream and `POST /api/tray/v1/open-console` ([contracts.md](contracts.md),
  "The tray app"). The status is built from what `/api/summary` and the
  Needs you queue (`crates/xenonet-state/src/needs_you.rs`) compute. The
  stream sends the status when it changes and an alert when an item
  starts or stops waiting.
- **Its credential.** There's no token file, since anything running as
  you could read it. The tray reads `runtime/service-<port>.json` as the
  Tools do, connects to the component socket as role `tray`, and the
  hello's answer carries the tray's key, new at each start of the service,
  only when the engine's process check passes: the process on the other
  end is `XenoNet Tray.app/Contents/MacOS/<executable>` by its real path,
  in `/Applications` or the home folder's `Applications`, with no agent or
  actor product above it. The routes take the key in `x-xenonet-tray`,
  never beside an `authorization` header, and the request must be yours
  too, so the key is no use from under an agent. The key is kept only in
  the tray's memory.
- **Its types** are written by hand in `tray/macos/Sources/TrayCore`, held
  to the schemas by fixtures: `test/fixtures/tray/` is checked by the JS
  and Rust tests and decoded by `swift test`.
- **A component of its own.** The tray is the `tray` component, versioned
  on its own (`components/versions.json`), one archive for both Mac CPUs.
  Its build leaves out debug info and remaps the checkout's and the home
  folder's paths, and the archive fixes modes, times and order, so two
  clean builds on one Mac give the same SHA-256. A new Swift toolchain on
  the release runner changes the bytes, and the release check then asks
  for a new tray version.
- **Installed by the bundle, put in place by `service install`.** The
  disk image's bundle installs the tray component. `xenonet service
  install` on macOS copies its `XenoNet Tray.app` to `~/Applications`,
  removes `com.apple.quarantine` from the copy, and writes and loads
  `~/Library/LaunchAgents/dev.xenonet.tray.plist`
  (`crates/xenonet-components/src/tray.rs`): started at login in your
  graphical session, again after a crash, not after you quit it.
  `uninstall` quits it and removes the agent and the app it set up.
- **Desktop steps aside.** The service tells a connected Desktop when a
  tray comes and goes, and Desktop on macOS hides its own tray while one
  is connected. One tray at a time.
- **Checking it without the screen.** `XENONET_TRAY_SNAPSHOT=<folder>`
  has the tray write its menu as text and its icon as a PNG whenever they
  change, and take a file called `choose` as a click on the item it names.

Windows and Linux have no tray app; Desktop keeps its own tray there.

## Security

- **More hours online.** An always-on service keeps the actor gateway and
  the network link up whenever the machine is, if the person turned them
  on. The tray shows the gateway's state.
- **Same local surface.** The service listens only on 127.0.0.1, and
  every route takes a credential ([authorization.md](authorization.md)).
  The tray has its own, scoped to the tray API.
- **Owner's actions stay the owner's.** Installing and uninstalling the
  service are refused from under an agent.

## Signing

macOS lists login items under the name of the signer of the bundle or
executable they run. XenoNet's Mac builds are signed ad hoc, with no team,
so Gatekeeper asks for approval on first open and Login Items can't show
a publisher. A Developer ID certificate fixes both, but a personal
certificate publishes its owner's legal name in every signature, so
XenoNet is to be signed only under an organization's name. The release
workflow supports Developer ID signing and notarization (`MAC_SIGN`), so
switching is configuration once that certificate exists. Until then the
tray is signed ad hoc like the console, and `SMAppService` registration,
which wants a signed bundle, isn't used.

## Not built

- The service checking for updates on its own and installing them at a
  safe moment.
- Updating the tray through `xenonet update` or Desktop's Updates panel.
- The tray's own controls: turning the gateway off, pausing notifications,
  starting at login, quitting XenoNet.
- Trays for Windows and Linux.
- The service starting before anyone signs in on Windows.

## Sources

- Apple, Service Management: [`SMAppService`](https://developer.apple.com/documentation/servicemanagement/smappservice),
  [`agent(plistName:)`](https://developer.apple.com/documentation/servicemanagement/smappservice/agent%28plistname%3A%29),
  [`register()`](https://developer.apple.com/documentation/servicemanagement/smappservice/register%28%29),
  [Updating helper executables from earlier versions of macOS](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos).
