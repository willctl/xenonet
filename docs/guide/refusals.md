# Provider refusals

When Anthropic or OpenAI refuses a turn in one of your Claude Code or Codex sessions, XenoNet notices, says so in the activity feed, and keeps a record of it on this computer, so you can look back at them or build a dataset from them.

## What counts

XenoNet reads refusals from the lines the apps write, never from what a model happened to say:

- **Claude Code** writes an assistant line whose `stop_reason` is `refusal`. Often it's a synthetic one (`isApiErrorMessage`) with the request's id and the API's `stop_details`: a category, such as `cyber`, and an explanation. Newer versions also write a `model_refusal_no_fallback` line, or `model_refusal_fallback` when another model answered instead. XenoNet keeps that as `refusal-recovered`.
- **Codex** writes an `error` event whose `codex_error_info` names a policy (`cyber_policy`, `bio_policy`, `misalignment_policy_violation`). XenoNet also keeps a `model_reroute`, when Codex moved a turn to another model, as `reroute`, and an `invalid_prompt` as `possible-refusal`.

A refused turn ends there, so the session stops looking like it's thinking. A new refusal is a line in the activity feed. Up to six an hour, at most one per session every ten minutes, it's also a notification.

## What XenoNet keeps

Each refusal gets a folder in XenoNet's state folder, under `refusals/`, with `record.json`: the provider, the app, the session, the model, the provider's category and words, the request's id, and where the transcript is.

With **keeping context** on, each new record also gets:

- `context.jsonl.gz`: the transcript's own lines up to the refusal. For Claude Code that's the chain of messages back to the last compaction. For Codex it's the session's settings, its turn contexts and the items since its last compaction.
- `request.json`: the same as a request would carry it, ready to tokenize. For Claude it's `{ provider, model, system, messages }`, and for Codex `{ provider, model, effort, instructions, input }`.

Some of what the model was given isn't in a transcript, and the record lists it under `missing`: the system prompt and tool definitions Claude Code sends, and what a provider adds on its side. Very long tool output is cut to its first and last 10,000 characters, and the record says so.

Keeping context is off until you turn it on. Transcripts can hold anything that was typed or read into them, secrets included, and XenoNet keeps these copies until you delete them. Turn it on with:

```bash
xenonet refusals context on
```

or with **Keep or stop keeping full context with provider refusals** in the palette. `XENONET_REFUSAL_CONTEXT=on` or `off` overrides both.

## Pulling them out

```bash
xenonet refusals                       # the records, newest first
xenonet refusals show <id>             # one record; --context prints its context
xenonet refusals export --out refusals.jsonl --with-context --scrub
xenonet refusals capture --all         # add context to records kept without it
xenonet refusals scan --since 90d      # look through transcripts for older ones
```

`export` writes one JSON line per refusal. Each line holds the record and, with `--with-context`, its `request` with the messages. `--scrub` replaces home folders with `~` and takes out anything that looks like a key. `capture` works only while the transcript is still there: Claude Code deletes its own after about a month. `--since` and `--provider` (`claude`, `codex`) narrow any of them.

These commands read the files themselves, so they work with XenoNet stopped. Nothing is sent anywhere.
