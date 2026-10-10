# Install

[Download XenoNet](https://github.com/willctl/xenonet/releases/latest) for Windows or macOS. Choose the bundle for an offline install. Desktop-only downloads fetch missing components on first launch. Neither needs a separate Node.js installation.

## Updates

Open **Check for XenoNet updates** in the command palette or **Updates...** in the tray or Mac app menu. XenoNet checks after startup and every six hours. Windows downloads Desktop updates in the background and waits for you to choose **Restart and update**. Restarting Desktop keeps the service, terminals and MCP processes running. You can turn off background checks and check manually instead.

The bundle installs Desktop, Service and Tools on first launch. They update separately after that. If a service update finds running terminals, close them and run the update again. Tools updates apply to new commands and MCP processes. [Components and updates](components.md) covers headless installs and the compatibility checks.

Unsigned Mac builds show that a new release is available and link to its download. Replace XenoNet in Applications to update it. Your settings stay in place.

## Windows

`npm run dist:win` builds `release/XenoNet-Setup-<version>.exe`, a standard Windows installer. By default it installs XenoNet for you only, in `%LOCALAPPDATA%\Programs\XenoNet`, without an admin prompt. It adds a Start menu entry, an optional desktop shortcut, and an entry in Installed apps.

The wizard asks who to install for, where, and whether to add a desktop shortcut and start XenoNet at sign-in. Both are on for a fresh install. If XenoNet is running, Setup closes it first. Reinstalling keeps your shortcut and sign-in choices.

Setup also adds `%LOCALAPPDATA%\XenoNet\bin` to your user Path, so the [xenonet command](cli.md) works in a new terminal once XenoNet has started. Uninstalling with your data removed takes it out again.

To uninstall Desktop, use Installed apps, or `Uninstall XenoNet.exe` in the install folder. It keeps Service, Tools and your data by default. Choosing to remove data also stops the service and removes the installed components.

The installer isn't code-signed yet, so SmartScreen may ask you to confirm the first time. Choose More info, then Run anyway.

### Silent install and uninstall

`XenoNet-Setup-<version>.exe /S` installs XenoNet for the current user, with Start menu and desktop shortcuts, starting at sign-in, and with the xenonet command on the user's Path. It doesn't start the app. Add these options after `/S` to change what it does:

| Option | Effect |
| --- | --- |
| `/NODESKTOP` | Skips the desktop shortcut. |
| `/AUTOSTART` | Starts XenoNet at sign-in, hidden in the tray. A fresh install does this unless you add `/NOAUTOSTART`. |
| `/NOAUTOSTART` | Doesn't start XenoNet at sign-in. |
| `--force-run` | Starts XenoNet when Setup finishes. |
| `/allusers` | Installs for all users, in Program Files. Run it elevated. |
| `/D=C:\Apps\XenoNet` | Installs in the folder you name. It must be the last argument. |

To uninstall silently and keep your data, run:

```bat
"%LOCALAPPDATA%\Programs\XenoNet\Uninstall XenoNet.exe" /S
```

Add `/PURGE` after `/S` to delete the layout, alerts, search index and settings too.

## macOS

`npm run dist:mac` builds a disk image and an installer package for each kind of Mac, in `release/`. The `arm64` files are for Apple silicon, and the `x64` files are for Intel.

To install from the disk image, `XenoNet-<version>-arm64.dmg`, open it and drag XenoNet onto Applications. If you open XenoNet from somewhere else, such as the disk image or Downloads, it offers to move itself into Applications.

The installer package, `XenoNet-<version>-arm64.pkg`, is a guided installer. It asks who to install for. Everyone on the Mac means `/Applications`, which needs an administrator password. Just you means `~/Applications`, which doesn't. If XenoNet is running, the installer closes it first, and it opens XenoNet when it's done.

The first time XenoNet opens, it asks whether to open at login, waiting in the menu bar. You can change that later from the menu bar icon or the XenoNet menu.

Closing the window keeps XenoNet running in the menu bar, the way Windows keeps it in the tray. The Dock icon shows only while the window is open. Reinstalling keeps your login choice.

To uninstall, choose Uninstall XenoNet… from the XenoNet menu. It asks whether to remove your data too, and keeps it by default. Then it moves XenoNet to the Trash.

### Opening XenoNet the first time

The app isn't signed or notarized by Apple yet. So the first time you open it, macOS says it can't verify that XenoNet is free of malware. To open it anyway:

1. Click Done.
2. Open System Settings, then Privacy & Security.
3. Scroll down to Security, and click Open Anyway next to the message about XenoNet.
4. Confirm.

The `.pkg` works the same way. You do this once for each new version.

### Command-line install and uninstall

| Command | Effect |
| --- | --- |
| `sudo installer -pkg XenoNet-<version>-arm64.pkg -target /` | Installs for all users, in `/Applications`. It doesn't start the app. |
| `installer -pkg XenoNet-<version>-arm64.pkg -target CurrentUserHomeDirectory` | Installs for the current user, in `~/Applications`, without admin. |
| `/Applications/XenoNet.app/Contents/MacOS/XenoNet --uninstall` | Uninstalls silently and keeps your data. If XenoNet is running, it quits first. |
| `/Applications/XenoNet.app/Contents/MacOS/XenoNet --uninstall --purge` | Uninstalls, and deletes the layout, alerts, search index and settings. |

### What's different on a Mac

- XenoNet checks repos with git. If git comes from Apple's Command Line Tools and they aren't installed, XenoNet leaves repo status off, rather than opening the tools' installer every 20 seconds. Run `xcode-select --install`, or install git with Homebrew, and restart XenoNet.
- Opened from Finder, apps get a bare `PATH`. XenoNet reads your login shell's `PATH` once at startup. So XenoNet and its terminal find `codex`, Homebrew and anything else your shell profile sets up.
- macOS asks before any app reads Documents, Desktop, Downloads or external drives. XenoNet asks only if your projects or sessions live there, or when you work there in its terminal.
