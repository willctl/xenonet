# Decision models

XenoNet occasionally needs a judgment its own rules can't make, such as which folders in your projects folder are really projects. Until now it asked Codex, which takes a while and writes its answer as text. A decision model answers typed questions instead: yes or no, one of a few options, or a place on a scale, each with how sure it is. XenoNet keeps only the answers it's sure enough of, and its own rules decide the rest.

XenoNet uses Jev by default, TypeSafe's decision model on [OpenRouter](https://openrouter.ai/docs/guides/community/jev). It costs a fraction of a cent per question. `XENONET_DECISION_MODEL` picks another.

## Setting it up

1. Make an OpenRouter key, and give it a credit limit.
2. In XenoNet, choose **Set the OpenRouter key for decision models** in the palette and paste the key, or run `xenonet decide key` and paste it there.

XenoNet keeps the key in Windows Credential Manager, the macOS keychain or the Secret Service on Linux, and never shows it again. `xenonet decide forget` and the same palette command forget it.

## What it decides

- **Which folders are projects.** In the projects panel, **Show what would be sent** now offers **Send to jev** beside **Send to Codex**. Jev gets the same outline you see: each folder's path inside your projects folder, its well-known files, counts and file types, and how many sessions ran there. Each folder is one question (project, a group of projects, or just a folder), and XenoNet keeps a pick only when Jev is at least 75% sure and the pick changes something. Your own rules still win.

More decisions will follow, each with what it sends shown first, and none of them on by default.

## What leaves this computer

Only what you're shown, and only when you send it. XenoNet asks OpenRouter for zero data retention and sends no user or session ids. Each decision is a line in XenoNet's log (`xenonet logs --all --code decide.`) with the model, how long it took and what it cost, never what was asked.

With the key, Token use also shows what it has spent today, this week and this month, and how much of its limit is left. XenoNet reads that from OpenRouter at most once a minute while a window is open, and sends the key only to OpenRouter.
