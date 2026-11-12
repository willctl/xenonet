# Development roadmap

Xenolith stores notes durably, files them into Markdown pages, and lets connected apps retrieve the supporting records.

## Implemented

- A shared inbox, deduplication, journaled curation and page history.
- Section search with filters, aliases and multiple query wordings.
- Optional hosted embeddings, fused with lexical search.
- Provenance labels, held changes, approval controls and conditional undo.
- Read-only Ask workers, scheduled page review and requested redaction.
- Rust programs for the service, CLI, native tray, window and installer.
- Spaces as signed chains with one curator each, grants as Biscuit tokens, and sync between machines on `xenolith/sync/2`: invites and codes (join/1), members holding the curator's feed, and shared mode over XenoNet's forward ([sync design](sync-design.md)).
- The upgrade from 1.x, dry run first and undoable, and `xenolith doctor` for what 1.x left.
- A versioned folder format ([FORMAT.md](../FORMAT.md)) for programs that read the folder, CI of its own on Linux, macOS and Windows, and version tags (`xenolith-v<version>`) with a release workflow.

## Remaining work

- Add authenticated local transport before claiming isolation between OS accounts.
- Review native UI dependencies and remove obsolete transitive packages as upstream support permits.
- Expand synthetic retrieval and poisoning cases without adding private user data.
- What the [sync design](sync-design.md#not-built-yet) lists as not built yet.

## Sequence

Make each change independently verifiable. Reproduce a failure, fix its shared cause, and run the relevant behavior suites. Keep machine setup, sign-ins, private evaluation questions and evidence outside the repository.

See [platform architecture](cross-platform-plan.md), [Rust runtime](rust-plan.md), [sync](sync-design.md), [search](search-plan.md), [memory evaluation](memory-eval.md), and [security](../SECURITY.md).
