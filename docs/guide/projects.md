# Finding projects

XenoNet reads your projects folder, which is `~/Projects` unless you [choose another](#changing-the-projects-folder). It finds projects there in two steps.

First, XenoNet walks down the folder the way ghq walks its root. It stops at the first repo it meets, and looks into every other folder, breadth first.

Then it works up from each folder a session ran in, the way git finds the repo you're in. This catches repos the walk didn't reach.

## What each folder becomes

- A folder with its own repo is a project. XenoNet doesn't look inside it, except for its submodules, up to 10, and for repos inside it where a session ran. A folder has its own repo when it holds a `.git` folder or file, or another version control system's marker, such as `.hg`, `.jj`, `.sl` or `.svn`.
- A folder without a repo that holds a project somewhere below it is a group. The map draws a group as a hollow frame, with its projects around it. Sessions run in the group's own folder land on it.
- Any other folder is a project if it's at the top level, if it has a workspace definition, or if it's inside a group and a session ran in it. Workspace definitions are `pnpm-workspace.yaml`, `workspaces` in `package.json`, a Cargo `[workspace]`, `go.work` and `[tool.uv.workspace]`, and the workspace files of Nx, Turborepo, Lerna, Rush and Bazel.

Repos are the unit. A monorepo in one repo is one project. A folder of separate repos is a group, even when it declares a workspace over them.

A worktree inside its repo, such as one in Claude's `.claude/worktrees` folder, belongs to that repo. A worktree checked out beside its repo is a project that orbits its repo.

## Limits

The walk goes 3 levels deep and reads at most 2000 folders. `XENONET_PROJECTS_DEPTH` and `XENONET_PROJECTS_MAX_DIRS` change these limits.

The walk skips dot folders and `node_modules`. Below the top level, it also skips other dependencies and build output, such as `vendor`, `dist`, `build` and `target`.

XenoNet leaves out a group below the top level that holds just one thing and has no sessions of its own. So `github.com/me/tool` doesn't draw two empty groups.

## Your rules

Your rules come first. Select a group or a project, or a session for the folder it ran in. Then use the palette to make that folder one project, look inside it for projects, or leave it out. The palette's commands for this include `Treat … as one project`, `Look inside … for projects` and, on a session, `Make … a project`.

`How XenoNet finds your projects` in the palette lists your rules, with an undo for each. It also says when the folders were last scanned.

## Asking Codex which folders are projects

For folders with no repo and no sessions, such as a folder of examples, the rules above can only guess. You can ask Codex instead, with `Ask Codex which folders are projects` in the palette. First, XenoNet shows you everything Codex would get:

- each folder's path inside the projects folder
- the well-known files in it, such as `package.json` and `README.md`
- how many files and folders it holds
- its commonest file types
- how many sessions ran there
- XenoNet's own call on it

A repo goes as just its path. Codex gets no other file names, nothing from inside a file, and no transcripts. XenoNet runs `codex exec` only when you press Send. It runs with your sign-in, in a Codex home of its own, without your plugins, hooks or instructions, and without a shell or web search.

Codex's picks can make a folder a project or a group, but they never hide anything, and your rules win over them. XenoNet keeps the picks with a fingerprint of the folders they were based on. It tells you when those folders have changed since, and it never asks Codex again by itself.

Asking Codex spends your Codex usage. Like your rules and the folder itself, asking Codex and `Forget Codex's picks` are yours to do. On macOS, XenoNet refuses them from under a session or an [actor product](actors.md). [Telling sessions apart on macOS](privacy.md#telling-sessions-apart-on-macos) says how XenoNet knows.

## Changing the projects folder

`Change the projects folder…` in the palette shows the folder XenoNet reads, and where that choice comes from. To read another folder, pick it with `Choose…` in the desktop app, or type its path, such as `~/code`.

As you go, XenoNet checks the folder. It says how many projects and groups it would find there, or why it can't use the folder. The folder has to exist and be readable. It can't be your home folder, a folder that holds your home folder, or the whole disk. If you give a link, XenoNet judges the folder the link leads to. XenoNet also warns you if you pick a repo, since it would show the repo's own folders as projects.

`Use this folder` reads the folder at once, and XenoNet keeps your choice in its `state.json`. If you reached the folder through a link, XenoNet keeps the folder the link leads to, since that's how sessions record it.

With XenoNet running, the `xenonet` command does the same:

- `xenonet root ~/code` reads another folder.
- `xenonet root --check ~/code` only checks it.
- `xenonet root --reset` goes back to `~/Projects`.

Changing the folder is yours to do, from XenoNet's window or your own terminal. On macOS, XenoNet refuses a change from under a session or an actor product.

When `XENONET_PROJECTS_DIR` is set, it decides the folder instead, and the panel says so.

### What happens when the folder changes

XenoNet scans the new folder, reads its repos' status and git identities, and redraws the map, all in one pass. Transcripts, sessions and the engine they use keep running. Sessions that ran outside the new folder show as folders outside it.

Your layout stays. XenoNet saves positions by each folder's full path, so going back to a folder brings its layout back.

Your rules stay too, kept by full path. Each rule applies whenever its folder is inside the one XenoNet reads. `How XenoNet finds your projects` lists them all, marks the ones outside the folder in use, and can undo any of them. `Clear your rules` there clears only the rules for the folder in use. `Clear rules for every folder` clears the rest too.

Codex's picks stay with each folder, and XenoNet never applies them to another folder. Each folder's picks say when its folders have changed since Codex looked.
