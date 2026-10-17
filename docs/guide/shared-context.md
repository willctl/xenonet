# Shared context (Xenolith)

Xenolith is XenoNet's shared context: one plain-Markdown wiki that every
session on your machine reads and writes through a local MCP server, with a
curator that files the notes sessions leave into pages. It lives in
[`xenolith/`](../../xenolith/README.md) in this repository and also runs on
its own. Its README covers installing it, its tools, spaces, access, tokens,
sync between machines and the audit chain.

## On XenoNet's map

XenoNet draws the wiki as a constellation of its own: one node for the wiki,
one per page, `[[links]]` between pages, and lines to the projects a page
mentions. Sessions' calls to the `xenolith` MCP server beam to the wiki, and
a `context_log` or `context_upsert_page` that names pages beams to those
pages.

XenoNet finds the folder the way Xenolith does: `XENOLITH_DIR`, then
`wikiDir` in Xenolith's `config.json` (in `~/Library/Application Support/Xenolith`
on macOS, `%APPDATA%\Xenolith` on Windows, `~/.config/xenolith` on Linux),
then `~/Xenolith`. Without Xenolith it looks for Agent Wiki, which Xenolith
came from, the same way. `XENONET_WIKI_DIR` overrides both. A folder whose
`PROTOCOL.md` is Xenolith's shows as Xenolith; any other shows as Agent Wiki,
as before.

XenoNet only reads the folder. Writing goes through Xenolith's own server,
under its access rules, so XenoNet never changes your wiki.
