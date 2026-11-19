# Eidolon release notes

## 0.1.0 (unreleased)

What you can do.

- Keep a folder of a cell, or a Firefox profile with its sign-ins, as one sealed eidolon with `eidolon new`, `seal` and `open`.
- Move an eidolon to another of your machines with `eidolon move <eid> -- ssh tower eidolon receive`. Only the chunks that changed cross, and only one machine can open it at a time.
- Sign in to a site yourself with `eidolon view <eid>` in Screen Sharing. Automation is off while the window is open.
- Give an agent the browser with `eidolon mcp <eid>`, or serve it on 127.0.0.1 with `--http` and publish it from XenoNet's **Network settings › Publish a service** for your other machines.
- Check an eidolon with `eidolon verify <eid>`.

What to know before you install.

- It needs [Adytum](../adytum/RELEASES.md) and Lima 2.1.2 for the cells. It builds from this repository with `npm run cargo -- build -p adytum-cli -p eidolon-cli`. There is no installer yet.
- Keys are kept in owner-only files under `~/.eidolon/keys`, because this platform's key store isn't used yet. Anyone who can read your files as you can read them.
- A site that ties a session to one device asks you to sign in again after a move.
- Moving to another machine needs a command that reaches it, such as `ssh`. Moving as a XenoNet request isn't built.
- Eidolon doesn't hide automation from sites. A site may ask for a CAPTCHA, and you answer it in `eidolon view`.
- There is no Windows or WSL driver.

What you must do.

- Run `eidolon machine init --name <name>` on each machine, and list every machine that may hold an eidolon when you make it, because the list is fixed at `eidolon new`.
- After a move that was cut off and left the sender closed, settle it with `eidolon move-settle`.
