# Keeping the service running

XenoNet's service is what lets sessions message each other, claim files and hear about edit collisions. Desktop starts it when it opens, and it runs until you log out. You can have it start at login instead, and start again if it stops unexpectedly. Then sessions keep coordinating while Desktop is closed, and after a restart before you open Desktop. It's off until you turn it on.

This works on macOS, Linux and Windows.

## Turning it on and off

- **In Desktop.** The first time installed XenoNet opens, it asks "Keep XenoNet running in the background?" Start at login turns it on. Not now leaves it off, and Desktop doesn't ask again. After that, **Background service** in the palette, the tray menu or the XenoNet menu on a Mac has the switch.
- **In a terminal.** `xenonet service install` turns it on, and `xenonet service uninstall` turns it off. `xenonet service status` says how it stands.

Turning it on or off is yours to do. XenoNet refuses it from under an agent such as Claude Code or Codex. Agents that drive the console can open Background service and read it, but not flip the switch.

Turning it off stops the copy that started at login, and Desktop starts its own again. It's refused while that copy has terminals open. Close them first, or use `xenonet service uninstall --force`.

## What it sets up

### macOS

`xenonet service install` writes a launch agent, `~/Library/LaunchAgents/dev.xenonet.service.plist`, and loads it with `launchctl bootstrap gui/<your uid>`. The agent runs `xenonet service run` from the launcher in XenoNet's state folder (`runtime/launcher/xenonet`), which stays where it is when the service and tools update.

- It starts when you log in.
- launchd starts it again when it crashes or exits with an error, at most once every 10 seconds.
- `xenonet service stop` stops it until you next log in. launchd doesn't restart a clean stop.
- Its output goes to the service's log in the runtime folder, `service-47840.log`, as it does when Desktop starts it. `xenonet logs` lists its errors.

macOS lists it in System Settings › General › Login Items & Extensions. Turning it off there keeps it from starting at login. A launch agent starts at login, not at boot: a Mac that should serve as a host before anyone logs in needs automatic login.

### Linux

`xenonet service install` writes a systemd user unit, `~/.config/systemd/user/xenonet.service` (or under `XDG_CONFIG_HOME`), and runs `systemctl --user enable --now xenonet.service`.

- It starts when you log in.
- systemd starts it again 5 seconds after it fails, and gives up after 10 starts in 5 minutes.
- `xenonet service stop` stops it until you next log in, or until `systemctl --user start xenonet`.
- Its output is appended to the service's log in the runtime folder, `service-47840.log`.

To start it at boot, before anyone logs in, which a host needs, run `loginctl enable-linger $USER` once. That may need an administrator, so XenoNet prints the command rather than running it.

### Windows

`xenonet service install` creates a scheduled task, **XenoNet Service**, in Task Scheduler, and runs it. XenoNet keeps the task's definition in `%LOCALAPPDATA%\XenoNet\xenonet-service.xml`. The task runs `xenonet service run` from the launcher in XenoNet's state folder (`runtime\launcher\xenonet.exe`), under a console host with no window, so nothing shows on your desktop.

- It starts when you sign in, as you, with your own rights. Creating it doesn't need an administrator.
- `xenonet service run` stays as the service's parent. It starts the service again 5 seconds after it fails, and gives up after 10 starts in 5 minutes. Task Scheduler starts the task again a minute after it fails to start, up to 3 times.
- `xenonet service stop` stops it until you next sign in.
- Its output is appended to the service's log in the runtime folder, `service-47840.log`.

Task Scheduler shows it in its library as XenoNet Service. Disabling it there keeps it from starting at sign-in, and XenoNet leaves it off until you run `xenonet service install` again. A scheduled task that runs as you runs only while you're signed in, so a Windows computer serves as a host only while someone is signed in to it.

Start at sign-in, in the tray menu and the installer, is a separate setting: it opens Desktop when you sign in. Uninstalling Desktop turns the scheduled task off too.

## How it works with Desktop

- When Desktop opens and no service answers, it asks launchd, systemd or Task Scheduler to start the registered one rather than starting a second copy. `xenonet service start` and `xenonet update service` do the same.
- If you turn it on while Desktop's service is running, the new one waits for that one to stop. When the running service has no terminals open, it stops right away, and the new one takes over within a few seconds. Desktop reconnects and reloads its window. With terminals open, the running service keeps going until it stops, at the latest when you log out.
- The menu bar icon, the tray and notifications come from Desktop. They show while Desktop runs.
- Uninstalling Desktop also turns this off, whether or not you remove your data.
- XenoNet Dev, run from a checkout, starts and stops its own service and never registers one. Its Background service panel says so.

## The settings it keeps

launchd, systemd and Task Scheduler start the service with a plain environment. The launch agent, unit or task keeps the XenoNet settings you had when you turned it on: `XENONET_PORT` (unless it's 47840), `XENONET_RUNTIME_DIR`, `XENONET_STATE_DIR`, `XENONET_CACHE_DIR`, `XENONET_PROJECTS_DIR`, `XENONET_GATEWAY_PORT`, `XENONET_ACTORS`, `XENONET_ASSISTANT`, `XENONET_HOSTS` and the `XDG_*_HOME` folders. If you change one, run `xenonet service install` again. A scheduled task has no environment of its own, so on Windows the task passes them to `xenonet service run` as arguments, such as `XENONET_PORT=47850`.

On macOS and Linux, `xenonet service run` adds your login shell's `PATH`, as Desktop does for the service it starts. So the service finds Codex, Homebrew's git and anything else your shell profile sets up.

## Checking on it

`xenonet service status` prints JSON:

- `running`: whether a service answers, with its version, process and terminals when one does.
- `supervised`: whether that service is the one launchd, systemd or Task Scheduler started.
- `login`: whether it starts at login. `registered` means the launch agent, unit or task definition is there, `loaded` means launchd, systemd or Task Scheduler has it, and `state`, `pid` and `lastExit` say how it's running. On Linux, `linger` says whether it starts at boot.

`xenonet doctor --only service`, and the doctor in the palette, check it under **Background service**:

- whether it starts at login;
- whether the launch agent, unit or task is what `xenonet service install` writes now, and the program it runs is there (if not, run `xenonet service install` again);
- whether launchd, systemd or Task Scheduler has it loaded, and if it's running;
- on Linux, whether lingering starts it at boot. That's a warning only where you have no graphical session, as on a host you reach over SSH, where the service would otherwise run only while you're logged in.

The supervisors have their own views: `launchctl print gui/$(id -u)/dev.xenonet.service` on a Mac, `systemctl --user status xenonet` on Linux, and `schtasks /query /tn "XenoNet Service" /v /fo list` on Windows.
