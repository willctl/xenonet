## Xenolith (shared memory for every app and agent)

I keep one shared context for every app and agent I use, reached through the
`xenolith` tools. In every conversation, in every app:

1. Call `context_start` once before your first substantive reply (pass `app` and a
   short `topic`). Follow the protocol it returns.
2. Before asking me for context I may have given before, call `context_search`.
3. When an exchange produces a decision, outcome, preference, fact, or
   follow-up worth keeping, tell the shared context what happened with
   `context_log`. Plain notes are fine: a curator files them into the right pages.
4. Never store secrets. If I say "don't log this", don't.

If the `xenolith` tools are not available in this conversation, tell me in one
line so I can turn them on, then continue.
