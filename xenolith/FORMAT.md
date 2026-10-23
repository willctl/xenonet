# Xenolith's folder format, version 1

Xenolith keeps shared context in one folder of plain files. This document is
the contract for programs that read that folder without going through
Xenolith's MCP server: a viewer, a search tool, a backup, a map of sessions.
Xenolith's own server and curator are the only writers. Read the folder; to
add to it, call the server (`context_log`, `context_upsert_page`) so notes are
queued, checked for secrets and filed.

The format has a version, in the marker below. Changes that only add (a new
frontmatter key, a new file) keep the version. A change that would make a
reader of an older version misread a file raises it. A reader that finds a
higher version than it knows should not trust what it parses, and should say
so rather than guess.

Paths below use `/`. Text files are UTF-8 with LF line endings; a reader
should also accept CRLF and a leading byte-order mark. Times are local, with
their offset: `2026-10-22T09:41:07-05:00`.

## Finding the folder

The first of these that applies is the folder:

1. `XENOLITH_DIR`, when set and not empty. A relative path is taken from the
   current directory. Xenolith creates the folder at its next start, so it may
   not exist yet.
2. `wikiDir` in Xenolith's `config.json`, when it is a non-blank string.
3. `Xenolith` in the home folder (`USERPROFILE` on Windows, `HOME` elsewhere).

`config.json` is in Xenolith's config folder, the first of:

1. `XENOLITH_CONFIG_DIR`, when set.
2. `XENOLITH_HOME`, when set (portable mode: everything in one folder).
3. The platform's place:
   - Windows: `%APPDATA%\Xenolith`
   - macOS: `$XDG_CONFIG_HOME/xenolith` when `XDG_CONFIG_HOME` is an absolute
     path, else `~/Library/Application Support/Xenolith`
   - Linux and other Unix: `$XDG_CONFIG_HOME/xenolith`, else
     `~/.config/xenolith`

When neither `XENOLITH_HOME` nor `XENOLITH_CONFIG_DIR` is set and that
`config.json` doesn't exist, an older install's `~/.xenolith/config.json` is
read instead, if it exists.

## The marker

The folder's root holds `xenolith.json`:

```json
{
  "format": 1,
  "server": "xenolith"
}
```

- `format`: the version of this document the folder follows.
- `server`: the name of the MCP server that writes the folder, as the apps
  connected to it know it. Its tools are `context_start`, `context_search`,
  `context_read`, `context_log` and `context_upsert_page`.

Xenolith writes the marker at every start. A folder without one was last
opened by a release from before the marker. Its `PROTOCOL.md` starts with
`# Xenolith protocol`, unless the person edited that line, and it follows
format 1.

## Layout

```
xenolith.json            the marker (the root only)
PROTOCOL.md              the rules every connected session follows; the person may edit it
README.md                what the folder is
index.md                 generated from the pages' frontmatter; never edited by hand
pages/<slug>.md          one page per project, person, preference, decision, how-to, reference or topic
log/<YYYY>/<YYYY-MM-DD>.md   one day of the activity log
inbox/<note id>.md       notes waiting for the curator
spaces/<kind>/<name>/    the other spaces (the root only)
.history/                earlier versions of pages and of PROTOCOL.md
.curator/                the curator's queue state, journal, archive and audit records
.locks/                  write locks while a writer works; safe to ignore
.gitattributes, .gitignore
```

Everything under a folder or file whose name starts with `.` is Xenolith's
own bookkeeping. Only the parts this document names are part of the format.

### Spaces

The folder is one space: this machine's, `machine:local`. Every other space
is a folder with the same layout (without the marker and without `spaces/`)
at `spaces/<kind>/<name>/`:

- `kind` is one of `project`, `identity`, `actor` or `machine`.
- `name` matches `^[a-z0-9][a-z0-9._-]{0,63}$` and doesn't contain `..`.
- A space's id is `<kind>:<name>`, such as `project:atlas`.

A folder under `spaces/` whose name isn't a valid id, or that is a link, is
not a space.

## Pages

A page is `pages/<slug>.md`. A slug matches `^[a-z0-9][a-z0-9-]{0,79}$`.

A page starts with frontmatter between `---` lines, then a Markdown body that
starts with a `# Title` heading:

```markdown
---
title: Atlas
type: project
summary: The data platform rewrite
tags: ["atlas","postgres"]
aliases: ["data platform"]
created: 2026-10-02T10:00:00-05:00
updated: 2026-10-22T09:41:07-05:00
updated_by: curator
---

# Atlas

Postgres 17, logical replication to the reporting box. See [[db]].
```

| Key | Value |
| --- | --- |
| `title` | the page's title |
| `type` | `project`, `person`, `preference`, `decision`, `howto`, `reference` or `topic`; other values are allowed and grouped by name |
| `summary` | one line, may be empty |
| `tags` | a list of lowercase tags, without `#` |
| `aliases` | other names to search the page by: at most 12, each at most 40 characters |
| `created`, `updated` | times |
| `updated_by` | `curator`, or the app that wrote the page directly |

Other keys may appear; keep what you don't know. A missing `title` is the
first `#` heading, else the slug; a missing `type` is `topic`.

Frontmatter is a small YAML subset, one `key: value` per line:

- A value is a plain scalar, a JSON string (`"..."`), or a JSON array of
  strings (`["a","b"]`). Xenolith writes only these.
- Readers also accept a single-quoted string (`'...'`, with `''` for a quote),
  a flow list written by hand (`[a, 'b c']`), a block list (`- item` lines
  after `key:`), and a ` # comment` after a plain value.
- The frontmatter closes at the first line that is `---` or `...`.

## Links

`[[slug]]` links to a page, and `[[slug|text]]` shows other text. The slug
follows the page rule above. A link to a page that doesn't exist is allowed:
the page may come later.

## index.md

`index.md` is generated from every page's frontmatter, grouped by type:

```markdown
<!-- GENERATED by xenolith from page frontmatter. Do not edit: it is rewritten on every page change. -->

# Index

## Projects

- [[atlas|Atlas]] - The data platform rewrite
```

## The log

Each day is `log/<YYYY>/<YYYY-MM-DD>.md`, named by the writer's local date. It
starts with `# YYYY-MM-DD`, and entries are appended in time order. Fields in
an entry's first line are separated by ` · ` (a space, U+00B7, a space). An
app's name is lowercase letters, digits, `.`, `_` and `-`, at most 40.

A full entry is a heading line, then optional head lines, then a body:

```markdown
## 09:41 · claude-code · Moved Atlas to Postgres 17

tags: atlas, postgres  
pages: [[atlas]], [[db]]  
sources: note:2026-10-22_09-40-58-118-a1b2c3

Logical replication to the reporting box; cut over on Friday.
```

- Heading: `## HH:MM · <app> · <title>`.
- Head lines, each optional, in this order, before any body text:
  `tags: a, b`, `pages: [[slug]], [[slug]]`, and `sources: note:<id>, ...`
  (the notes the curator wrote the entry from). Each ends with two spaces
  when another follows.
- Body: Markdown, up to the next entry, day heading or comment line. Links in
  the body name pages too.

A compact entry is one line: `- HH:MM · <app> · <text>`. Links in the text
name the pages it is about. The curator, the window and sync write these,
such as `- 10:02 · curator · Change approved: Atlas [[atlas]]`.

A line `<!-- curator batch <batch id> -->` closes one curator batch: the
entries since the previous comment line came from it. Other comment lines may
appear and mean nothing to a reader.

Full entries are separated by a blank line; compact entries that follow each
other by a single line break.

## Notes

### Note ids

A note id is the local time it was submitted and six random characters:
`YYYY-MM-DD_HH-MM-SS-mmm-xxxxxx`, matching
`^[0-9]{4}-[0-9]{2}-[0-9]{2}_[0-9]{2}-[0-9]{2}-[0-9]{2}-[0-9]{3}-[a-z0-9]{6}$`.
Its month folder is `YYYY/MM`, from the id. `context_read` opens a note as
`note:<id>`.

### The inbox

A note waiting for the curator is `inbox/<id>.md`: frontmatter, then the
note's text.

| Key | Value |
| --- | --- |
| `id` | the note id (a file dropped in by hand may have none: the file name is its id) |
| `kind` | `log`, or `page` for a suggested page |
| `app` | the app that sent it |
| `source` | `user`, `observed`, `external` or `agent` (the default); only the person's own notes are `user` |
| `by` | who submitted it, as the server knew it: `owner` or `cli` (the person), `local:<client>` (a program on this machine), or a token's subject such as `machine:laptop` |
| `title` | one line |
| `tags`, `pages` | lists |
| `submitted` | a time |
| `transport`, `client` | how it came: `stdio`, `http` or `sync`, and the client's name |
| `page_slug`, `page_title`, `page_type`, `page_summary` | for a page note: the page it suggests |
| `page_mode` | `append`, `replace` (the whole page), or `delete` (a page removed in a followed space's copy) |
| `page_base` | for `replace` and `delete`: the hash of the page it was written against, or `none` |

A page hash is the first 16 lowercase hex digits of the SHA-256 of the page
file's text, with CRLF line endings read as LF.

### A note's status

Each note is in one place, which is its status:

| Where | Status |
| --- | --- |
| `inbox/<id>.md` | waiting: pending, retrying or failed, as below |
| `.curator/done/YYYY/MM/<id>.md` | filed by the curator, with its record in `.curator/audit/YYYY/MM/<id>.json` |
| `.curator/sent/YYYY/MM/<id>.md` | in a space this machine follows: sent to the space's curator on another machine |
| `.curator/sync/dead/<id>.md` | in a space this machine follows: refused for good by its curator |

A waiting note's queue state is `.curator/state/<id>.json`, when the curator
has tried it: `attempts` (a number), `lastError`, `nextAt` (when it is tried
again, in milliseconds since 1970), and `dead: true` once it failed for good.
No file, or `attempts` 0, is pending; `attempts` above 0 is retrying; `dead`
is failed.

A sent note's file starts with one line before its frontmatter,
`<!-- sent to <peer> at <time>: <the curator's answer as JSON> -->`, and a
refused one with `<!-- refused by <peer> at <time>: <reason> -->`.

`context_read` of `note:<id>` puts one status line before the note's text:

- `Pending: the curator has not filed this note yet.`
- `Filed by the curator at <time>: <disposition> (<reason>); pages changed: [[slug]], [[slug]].`
  (the parts it has)
- `Sent to the space's curator on another machine, which files it there.`
- `Refused for good by the space's curator on another machine; the first line says why.`

## What this format leaves out

Who may read or write which space (`access.json`), tokens, sync settings and
the audit chain live in Xenolith's config and state folders, not in this
folder, and are not part of this format.
