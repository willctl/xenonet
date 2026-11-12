# Shared context (Xenolith)

Xenolith is XenoNet's shared context: one folder of plain-Markdown pages
that every session on your machine reads and writes through a local MCP
server, with a curator that files the notes sessions leave into pages. It
lives in [`xenolith/`](../../xenolith/README.md) in this repository and runs
on its own, with or without XenoNet. It builds on XenoNet's library crates
(`xenonet-id`, `-log`, `-authz`, `-net`, `-model` and `-os`, with `-text`
and `-keys` beneath them) and never on the app: not the service, the
surfaces or the adapters. Its README covers installing it, its tools,
spaces, access, tokens, sync between machines and the audit chain, and
[its sync design](../../xenolith/docs/sync-design.md) how its spaces sync.

## On XenoNet's map

XenoNet draws the shared context as a constellation of its own: a node
named Xenolith (Shared context in the legend), one node per context page,
`[[links]]` between pages, and lines to the projects a page mentions.
Sessions' calls to the `xenolith` MCP server beam to the Xenolith node, and
a `context_log` or `context_upsert_page` that names pages beams on to those
pages.

Xenolith keeps pages for each space, such as a project, an identity, an
actor or another machine, in `spaces/<kind>/<name>/` under its folder (a
second space of that name, from another Xenolith, in `<name>~<six
characters>/`). Each space is laid out as the folder itself is. XenoNet
shows every space's pages in the constellation, with the space in a page's
panel. It ties a project space's pages to its project by the project's id
when the folder's marker, `xenolith-space.json`, is signed by the context key
Xenolith's grant names and the space's owner owns the project; a space about
a project someone else owns shows "about it, owned by" them, tied to
nothing; any other space is tied to the projects of its name
([the space marker](../../xenolith/FORMAT.md#the-space-marker)). A space is
about its project's id only once it's bound, which takes the person's
signature, and the window doesn't offer that yet, so spaces are tied by
their names. A `context_log` or
`context_upsert_page` with a `space` beams to that space's pages, never to a
page of the same name in this machine's own space. The unfiled count is
this machine's inbox.

It reads pages as Xenolith does. A link is `[[slug]]` or `[[slug|text]]`. A
page without a `title` takes its first top-level heading, and without one its
file name. Its `type` is read in lowercase. Its `aliases` show in its panel, and
the palette finds the page by them.

## Finding and reading the folder

XenoNet reads the folder by the format Xenolith documents in
[`FORMAT.md`](../../xenolith/FORMAT.md), and finds it as that file says:
`XENOLITH_DIR`, then `wikiDir` in Xenolith's `config.json`, then
`~/Xenolith`. Xenolith makes the folder either of the first two names when
it next starts, so XenoNet takes that folder even before it's there. The
`config.json` is in `XENOLITH_CONFIG_DIR` when it's set, else in
`XENOLITH_HOME`, else in the platform's place:

- macOS: `$XDG_CONFIG_HOME/xenolith` when `XDG_CONFIG_HOME` is an absolute
  path, else `~/Library/Application Support/Xenolith`
- Linux and other Unix: `$XDG_CONFIG_HOME/xenolith` when `XDG_CONFIG_HOME`
  is an absolute path, else `~/.config/xenolith`
- Windows: `%APPDATA%\Xenolith`

When neither `XENOLITH_HOME` nor `XENOLITH_CONFIG_DIR` is set and that
`config.json` isn't there, an older install's `~/.xenolith/config.json`
counts. Without Xenolith, XenoNet looks for Agent Wiki, which Xenolith came
from, the same way, taking only a folder that's there. `XENONET_WIKI_DIR`
overrides both.

Xenolith writes a marker, `xenolith.json`, at the folder's root each time it
starts. The marker names the MCP server that writes the folder and the
version of the format the folder follows. When XenoNet starts, the shared
context takes the server the marker names, so if you registered Xenolith
under another name, sessions' calls to it still beam to the Xenolith node
and the pages they name. Calls to `xenolith` and `agent-wiki` always count.

A folder without a marker (Agent Wiki's, or one only an older Xenolith has
opened) belongs to the server whose lookup found it, so a folder you
moved keeps its server and the beams to it. For a folder `XENONET_WIKI_DIR`
names, XenoNet checks whether Xenolith's lookup or Agent Wiki's names that
folder. Only when neither does it read the folder's `PROTOCOL.md`, which you
can edit: one that starts as Xenolith's shows as Xenolith, and any other as
Agent Wiki. A marker XenoNet can't read, such as one that isn't JSON or has
no `format`, counts as no marker.

XenoNet reads version 1 of the format, and checks the marker each time it
reads the folder. When the marker gives a later version, the folder may have
changed in ways XenoNet would misread, so XenoNet doesn't guess at it. The
Xenolith node stays on the map, but XenoNet shows none of its pages, log or
inbox. The node's panel says why, and the status panel's list of sources
shows the shared context as not read. A XenoNet that reads that version
shows them again.

XenoNet only reads the folder. Writing goes through Xenolith's own server,
under its access rules, so XenoNet never changes your shared context. Like
Xenolith, it leaves out any page, log or note that's a link, and any folder
of them that is: a link below the folder isn't part of the shared context.
The folder itself can be a link.

## Syncing over your network

Xenolith syncs its spaces between machines on its own, by invite
([its README](../../xenolith/README.md#sync-between-machines)). On a machine
in a XenoNet [network](network.md), it can sync over XenoNet's connections
instead, with the network's members, and needs no invite for them. XenoNet
carries Xenolith's protocol, `xenolith/sync/2`, for it
([forward(alpn)](../design/network.md#forwardalpn)), and only between members
of the networks you tick.

1. **Allow Xenolith.** When Xenolith starts on a machine where XenoNet runs,
   it asks XenoNet for a grant. The ask shows in **Network settings › Shared
   context** as "Xenolith asks to sync over the network". Tick the networks it
   may reach and choose **Allow**, which asks you (Touch ID on a Mac). The
   grant goes from XenoNet to Xenolith over a socket only your account can
   use, never through the window, a command or a log. It lasts 30 days and
   renews itself while Xenolith keeps coming back. **Turn down** asks
   nothing, and Xenolith asks again after it restarts. **Take back** ends
   Xenolith's sync over the network at once.
2. **Make it yours.** A space Xenolith made on its own belongs to its
   standalone owner key, which can't share into a network. Under the
   allowed grant, each space this machine's Xenolith curates is listed;
   **Make it yours** moves it to you, and asks you.
3. **Share into a network.** **Share into** *network* lets that network's
   members' Xenoliths read the space and leave notes in it, and asks you. The networks
   the space's project is already shared into come first, marked "(from the
   project)". A space shared into one network is never shown to a machine
   that's only in another.
4. **Stop sharing.** **Stop sharing into** *network* takes the space back
   from that network, and asks nothing. When you stop sharing a project into
   a network, Xenolith holds that project's space back from it at once, and
   the space shows **Stop sharing** marked, to record it.

For each of these, Xenolith drafts the change and XenoNet checks the draft
does exactly what the button says before your device signs it. XenoNet
reads what it carries for Xenolith, as the same account that reads the
folder anyway; Xenolith checks every peer itself, on its own copies of the
chains and each peer's signed hello.

On Windows, XenoNet's side of this is a named pipe that isn't built yet, so
a Windows machine syncs its spaces by Xenolith's own invite instead.
