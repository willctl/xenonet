# Identities

XenoNet links each session to the account it ran as, and each project to the accounts behind its repo. It reads all of this from what's already on disk.

## Claude accounts

XenoNet takes the signed-in Claude account, in its organization, from `oauthAccount` in `~/.claude.json`. It never takes the name fields there.

Claude desktop keeps each session in a folder named for its account and organization. Remote Control sessions record their owner in the transcript. Other Claude Code sessions get the account that was signed in when they last ran.

## ChatGPT accounts, for Codex

XenoNet reads the claims in the ID token in `~/.codex/auth.json`. The claims are your email, workspace and plan, and when you signed in. XenoNet never keeps the tokens.

Codex doesn't record the account on a thread, so a thread gets the account that was signed in when it last ran. A thread run on another provider gets none.

## GitHub accounts

XenoNet reads the accounts on each host, and the active one, from gh's `hosts.yml`. It never reads gh's tokens.

A session gets its GitHub accounts through its project. A project's owner comes from its origin remote, with SSH host aliases resolved. A project also has the account that a push over HTTPS signs in as. That's the user name in git's credential settings, or else gh's active account.

## git identity

XenoNet reads `git config user.name` and `user.email` in each repo, `includeIf` rules included. A session gets its git identity through its project.

## How sure a link is

Each link says how sure XenoNet is of it:

- recorded, when the app recorded it
- likely, when the account was signed in at the time
- assumed, when it's the only account XenoNet has seen
- your pin, when you pinned it

XenoNet keeps a timeline of the sign-ins it sees. So a session keeps its account after you switch, and an account you've signed out of keeps its email.

A folder without a remote takes the owner that its group's projects share.

XenoNet reads an account file again only when the file changes. It reads a repo's git config again only when that config or yours changes. `Detect identities again` in the palette reads everything afresh, and `XENONET_IDENTITIES=off` stops all of it.

## Naming and pinning identities

`Your identities` in the palette is where you name identities. Identities with the same name show as one chip. So a GitHub account and the email you commit with can be one "side project", and their projects stand on a ring in the name's color.

To pin a session or a project to an account that XenoNet couldn't place, or placed wrong, select it and use the palette.

## On the map

Each session shows the account it ran as, Claude or ChatGPT. Each project shows the GitHub account that owns it, the account a push signs in as, and the email its commits carry.

XenoNet flags a push that signs in as one of your GitHub accounts into a repo another of your accounts owns. It also flags commits under one of your names into a repo under another.

The chips under Filters show only one identity's work, and hovering a chip lights up that identity's sessions and projects. To mask identities for screenshots, see [What shows on screen](privacy.md#what-shows-on-screen).
