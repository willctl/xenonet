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

## Several folders for an app

Claude Code and Codex keep their sessions and their sign-in in one folder each (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`). People who keep accounts apart give each account a folder of its own. XenoNet reads each app's own folder, and another only once you name it:

```console
$ xenonet folders
$ xenonet folders add claude ~/.claude-work
$ xenonet folders remove claude ~/.claude-work
```

`xenonet folders` also lists folders beside your home folder that look like another one (`.claude-work`, `.codex-2`), and reads none of them until you add it. Naming a folder is yours: the command is refused when a session runs it. XenoNet reads the folders you named when its service starts, so stop and start the service after a change.

Each folder has its own sign-in file and its own timeline of the accounts it has held. A session found in a folder you named is linked to that folder's account by its folder, which counts as recorded and ranks above likely. If the folder has held more than one account, the session gets the one signed in when it ran. Hooks (`xenonet connect`) go into every folder you named, Claude Code's plan usage is read for each with its own `.credentials.json` (a folder whose token is only in the macOS keychain has no reading), and a session you resume, copy or wake from a folder runs in that folder with its sign-in.

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

## A project's rule

A commit or push under the wrong identity links accounts you keep apart, and it can't be undone. A rule says which git email a project commits with and which GitHub account a push signs in as. Set it in your own terminal, from the project's folder:

```console
$ xenonet identity rule --git nova@example.com --push nova-dev
```

`--push` takes `<login>` for github.com or `<login>@<host>` for another host. `--claude <email>` and `--chatgpt <email>` name the Claude Code and Codex account the project's sessions are to run as. Any part alone works, and a part you leave out stays as it was. `--mode refuse` is described under [Accounts](#accounts-and-what-xenonet-can-stop); the mode is `warn` unless you say so. `xenonet identity rule --clear` takes the rule away, and `xenonet identity rule` with no flags shows it and how it checks now. Setting a rule is yours: the command is refused when a session runs it. It also needs the folder to be in a project XenoNet knows. See [The xenonet command](cli.md).

XenoNet checks each rule against what it read of the project: the email its commits carry and the account a push signs in as. Each part comes out as ok, differs or unknown. Unknown means XenoNet couldn't read it, and that is never a mismatch.

## Accounts, and what XenoNet can stop

A rule's Claude Code or ChatGPT account is checked against the account the project's newest session of that kind ran as. A start XenoNet makes in the project, from your window, another machine, a delegation or an intent, runs in the folder that is signed in to the rule's account, with that folder's own sign-in (not the token XenoNet's runs share, whose account XenoNet can't tell). When no folder it reads holds the account, the start is refused and says so, in either mode. A start in a sandbox is refused too: a sandboxed run signs in with the shared token.

In `refuse` mode a Claude Code run XenoNet starts here also may not run `git commit`, `git push` or `gh`: XenoNet gives it Claude Code's `--disallowedTools` for them, which joins the deny rules of its settings. The rule is a prefix match, so a commit written another way (`git -C dir commit`, a script) isn't caught.

Everything else stays a warning, because XenoNet can't stop it: your own terminals, Claude desktop, a `/login` inside a running session, a push over SSH, and Codex threads.

## When a rule doesn't match

Only a part that differs counts. Then:

- Needs you gets an item for the project, "would commit or push as the wrong identity". It names no account, so it stays readable with identities masked. Show selects the project. A session that ran as another Claude Code or ChatGPT account than the rule names counts the same way.
- The project's inspector says which part differs and what git or a push would use instead.
- A session that starts in the project is told the rule and what git would use, and not to commit or push until it is fixed.
- `xenonet identity check` prints the check and exits 1.

This is a warning. XenoNet doesn't see a commit or push made by a program it doesn't read, a terminal of your own included, so nothing stops those except the hook below.

## The pre-push hook

`xenonet identity hook install` puts a git pre-push hook in the repo of the folder you run it in, or the folder you give it. It is off until you install it, and it checks only a project that has a rule. It refuses a push when:

- the account the push signs in as isn't the rule's. XenoNet reads that account for the project's origin remote, so a push to another remote is held to the same one.
- a commit in the push has an author or committer email other than the rule's. The hook lists the commits the remote doesn't have yet. A new branch is checked against what the remote already has, and a branch being deleted adds nothing.
- XenoNet can't be reached. You installed the hook to protect the repo, so it stops there and says why.

A project with no rule, or a folder outside any project, is let through. Each refusal says what is wrong and how to fix it: switch the account, set `user.email` and rewrite the commits, or change the rule.

`git push --no-verify` skips the hook. That override is yours, and every refusal says so.

The hook goes where git looks for hooks: `core.hooksPath` if it is set, else the repo's `hooks` folder. Linked worktrees share that folder, so one install covers them all. It never replaces a pre-push hook that isn't its own, and says so instead; a hook of its own carries a line that marks it. `xenonet identity hook remove` takes out only that hook.

