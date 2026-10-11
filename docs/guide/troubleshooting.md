# When something goes wrong

XenoNet keeps a log of what went wrong and checks its own setup every half hour. Codex or Claude Code on your computer can look at both with you.

## See what went wrong

```bash
xenonet logs
```

This lists the errors and warnings from the last day, from the background service, the desktop app and the `xenonet` command. Each line has a code that says what it is, such as `service.port-in-use` or `doctor.claude.auth`, and a fix when there is one. It reads the log files themselves, so it works when XenoNet is stopped or won't start.

- `--errors` shows errors only, and `--all` adds info lines.
- `--since 6h` (or `30m`, `2d`) looks further back or less far.
- `--summary` lists each code once, with how often it happened.
- `--code doctor.` keeps codes that start with that.
- `--dev` reads XenoNet Dev's log instead of the installed XenoNet's.

In the window, **Check XenoNet's setup** in the palette opens the doctor, which lists the last day's errors by code under its checks.

The log lives in XenoNet's state folder, in `logs/`: `service.ndjson`, `desktop.ndjson` and `cli.ndjson`, one JSON line per entry. Each file moves aside past 1 MB, and two older ones are kept. Home folders show as `~`, and anything that looks like a key or a token is taken out before it's written. Transcripts, prompts and session titles are never in it.

## Check the setup

```bash
xenonet doctor
```

This checks sign-ins, the folders XenoNet reads, its own parts, its launcher, whether Claude Code, Codex and Grok Build are connected and recent errors, with a fix for each problem. XenoNet runs the same checks every half hour. When a check finds a new problem, it goes in the log too, so you can see it later even after it clears.

## Get help from Codex or Claude Code

```bash
xenonet diagnose --ask
```

XenoNet writes what it can tell (the doctor's findings, its errors by code, where its hooks point and its versions) to a folder of its own in its state folder. Then it starts a Codex session there that reads it, together with XenoNet's own code, and talks it through with you.

- The session runs read-only and asks before it changes anything.
- It never files an issue, opens a pull request or sends anything. If it finds a bug in XenoNet, it writes `issue-draft.md` in that folder. Read it, and send it yourself if you choose.
- `--app claude` uses Claude Code instead, in plan mode.
- `--model <model>` picks the model. `XENONET_DIAGNOSE_APP` and `XENONET_DIAGNOSE_MODEL` set your defaults.

In the window, the doctor's **Ask Codex or Claude Code…** does the same in a terminal tab, and remembers the app and model you chose.

## Share what XenoNet can tell

```bash
xenonet diagnose
```

This prints the same summary as Markdown, ready to paste into an issue (`--json` for JSON, `--out <file>` to save it). The doctor's **Copy diagnostics** copies it. It holds no transcripts, titles, project names, identities or keys, and XenoNet refuses to make it if anything that looks private is left after scrubbing.

## Common problems

**Port 47840 is already in use.** Another program holds XenoNet's port. The log names it when it's another XenoNet, such as XenoNet Dev started on the wrong port. Quit that one, or start XenoNet with another `XENONET_PORT`.

**The doctor says "connected, but … is gone".** XenoNet's hooks run files that aren't there any more, often from a checkout that was moved or deleted. Run `xenonet connect`. Each XenoNet on the computer (the installed one, XenoNet Dev) keeps its own hooks and MCP server, so connecting one never removes the other's.

**The launcher can't find XenoNet's Tools.** The `xenonet` command and the MCP servers can't start, though hooks still reach XenoNet. Open XenoNet, which repairs them, or install XenoNet again.
