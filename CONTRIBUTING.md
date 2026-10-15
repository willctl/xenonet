# Contributing to XenoNet

Issues and pull requests are welcome. This page says how to run XenoNet from a checkout, how to check a change and how to send it. `AGENTS.md` says the same for coding agents, with how they can drive XenoNet to check their work. For a security problem, see [SECURITY.md](SECURITY.md) instead of opening an issue.

## Running from source

You need [Node.js](https://nodejs.org) 22.13 or later (CI uses 24) and git.

```bash
git clone https://github.com/willctl/xenonet
cd xenonet
npm install
npm run app    # the desktop app from source, as XenoNet Dev
npm run dev    # the server and hot-reloading page alone, at http://127.0.0.1:47842
npm run demo   # the same with made-up data, at http://127.0.0.1:47841
```

XenoNet Dev has its own settings, port and layout, so it runs beside an installed XenoNet, and it reloads as you save. Look at a change in the demo instance: its projects and sessions are made up, so nothing of yours ends up in a screenshot. [Development](docs/guide/development.md) covers the instances, driving XenoNet from a terminal or an agent, the VM lab and how the code is laid out.

## Checking a change

```bash
npm test            # unit tests, about half a minute
npm run test:e2e    # end-to-end tests on a hidden XenoNet Dev, about five minutes
npm run build       # the production page, which --prod and the installers use
```

Run `npm test` after every change. Run `npm run test:e2e` too when a change touches the page, Desktop or the control channel. It needs a GPU, since software rendering doesn't settle the map's tests reliably, and a failing test saves a screenshot of what it saw to `.xenonet/`. There is no linter: the tests are the check, and the code's style is the code around it, with plain comments that say why, no new dependency without a reason, and paths under the home folder shown as `~`.

The Checks workflow runs `npm test` and `npm run build` on Windows, macOS and Linux, and the updater's desktop tests on Windows. Hosted runners skip the Desktop restart test, so run it before a release; [Releases](docs/guide/releases.md) says how.

## Sending a change

1. Branch from `main`, with one logical change per branch.
2. Write commit messages as [Conventional Commits](https://www.conventionalcommits.org): a plain subject such as `feat(ui): ...`, `fix(engine): ...`, `docs: ...` or `test: ...`, then a body that says why.
3. Open a pull request against `main` that says what changed and how you checked it. The Checks workflow runs on it. A green run and a review are what merging takes.
4. Keep personal paths, real session names and screenshots of real sessions out of it. The demo instance is there for pictures.

By sending a pull request you agree that your change is under the [MIT license](LICENSE), like the rest of XenoNet.

## Writing copy

Text people read, in the console, the `xenonet` command, the guide and the release notes, follows two rules.

- XenoNet is "The network for SI". SI (super intelligence) belongs in the tagline and in the first sentence of a page or a release's notes. Everywhere else, say what the thing is: sessions, Claude Code and Codex by name, and each actor by its name. Never write "AI" or "artificial intelligence". Product names, command names and tool names stay as they are.
- Plain words, sentence case, no em dashes, colons only before a list, one idea per sentence, and a mechanism or a number in each claim. Say what the app does and what the person should do next. The [unslop guidance](https://github.com/cursor/plugins/blob/main/pstack/skills/unslop/SKILL.md) is the checklist.

## Release notes

Release notes live in `docs/releases/<version>.md`, one file per version, written for the person using XenoNet: what changed, any install limits and anything they must do. A change people will notice gets its line there, in the same pull request or in the release's own `chore: prepare XenoNet <version>` commit. [Releases](docs/guide/releases.md) has the whole release process.
