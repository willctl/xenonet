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

Xenolith keeps a wiki for each space, such as a project, an identity, an
actor or another machine, in `spaces/<kind>/<name>/` under the wiki's folder.
Each is laid out as the wiki is. XenoNet shows every space's pages in the
constellation, with the space in a page's panel, and ties a project space's
pages to the project of that name. A `context_log` or `context_upsert_page`
with a `space` beams to that space's pages, never to a page of the same name
in this machine's wiki. The unfiled count is this machine's inbox.

It reads pages as Xenolith does. A link is `[[slug]]` or `[[slug|text]]`. A
page without a `title` takes its first top-level heading, and without one its
file name. Its `type` is read in lowercase. Its `aliases` show in its panel, and
the palette finds the page by them.

XenoNet finds the folder the way Xenolith does: `XENOLITH_DIR`, even before
Xenolith has made that folder, then `wikiDir` in Xenolith's `config.json`,
then `~/Xenolith`. That `config.json` is in `XENOLITH_CONFIG_DIR` when it's
set, else in `XENOLITH_HOME`, else in `~/Library/Application Support/Xenolith`
on macOS, `%APPDATA%\Xenolith` on Windows and `~/.config/xenolith` on Linux.
Until Xenolith's installer moves it there, the older `~/.xenolith/config.json`
counts, unless `XENOLITH_HOME` or `XENOLITH_CONFIG_DIR` is set. Without
Xenolith it looks for Agent Wiki, which Xenolith came from, the same way.
`XENONET_WIKI_DIR` overrides both.

The wiki belongs to the server whose lookup found it, so a wiki you moved
keeps its server and the beams to it. For a folder `XENONET_WIKI_DIR` names,
XenoNet checks whether Xenolith's lookup or Agent Wiki's names that folder.
Only when neither does it read the folder's `PROTOCOL.md`, which you can
edit: one that starts as Xenolith's shows as Xenolith, and any other as
Agent Wiki.

XenoNet only reads the folder. Writing goes through Xenolith's own server,
under its access rules, so XenoNet never changes your wiki. Like Xenolith,
it leaves out any page, log or note that's a link, and any folder of them
that is: a link below the wiki's folder isn't part of the wiki. The wiki's
folder itself can be a link.
