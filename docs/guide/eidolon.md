# Eidolon, an identity's state that moves

Eidolon keeps the state of one identity, a folder of files or a signed-in browser, as one sealed thing. It opens in a cell behind the identity's wall and moves to another of your machines as a unit, and exactly one machine can open it at a time.

This page is for using Eidolon with XenoNet. [The README](../../eidolon/README.md) has the command list and the crates. Eidolon makes its cells with [Adytum](adytum.md), so read that page first.

## Setting up

Build both tools, check Adytum, and give this machine its keys.

```text
npm run cargo -- build -p adytum-cli -p eidolon-cli
adytum doctor
eidolon machine init --name desk
eidolon machine show > desk.holder.json
```

`eidolon machine init` makes the machine's signing key and sealing key and says where they are. This platform has no operating system key store that Eidolon can use yet, so the keys are owner-only files under `~/.eidolon/keys`. `eidolon machine show` prints the machine's holder entry, which is its public keys and its name. You give that file to the machines that will share an eidolon with it.

Everything lives under `~/.eidolon`, or under `EIDOLON_HOME` if you set it.

## A first eidolon with a folder

Make a cell, put a folder in it, and make the eidolon.

```text
adytum create adytum-a --sealed --wall wall_work
adytum start adytum-a
adytum exec adytum-a -- mkdir -p /tmp/notes
adytum cp ./notes/todo.txt adytum-a:/tmp/notes/todo.txt
eidolon new --wall wall_work --holder desk.holder.json --folder notes=/tmp/notes --leave-out cache
```

`eidolon new` prints the eidolon's id, `eid_` and 26 characters. A folder facet is a name and a path inside the cell. `--leave-out cache` leaves the `cache` folder out of what is sealed, and a pattern belongs to the facet written before it. The holder list is every machine that may ever hold this eidolon, and this machine has to be on it.

`eidolon new` makes the eidolon sealed and empty. The first `seal` takes the files from the cell.

```text
eidolon seal <eid> --cell adytum-a
eidolon verify <eid>
eidolon ls
```

## Opening it

```text
eidolon open <eid> --cell adytum-a
```

Open lays the files out in the cell, byte for byte with their modes. The cell has to be behind the eidolon's wall, or Eidolon refuses with `wrong_wall`. An open eidolon is open in one cell, and a second open elsewhere is refused with `second_seat`. To save changes, run `eidolon seal <eid>`, with no `--cell`, from the machine that has it open.

## Moving it to another machine

A move needs a way to start `eidolon receive` on the other machine and a byte stream to it. `ssh` is the usual one.

```text
eidolon seal <eid>
eidolon move <eid> -- ssh tower eidolon receive
```

Everything after `--` is the receiving side's command, run as an argument list. The two machines then do this.

1. `desk` offers the names of the chunks it holds. `tower` answers with the names it lacks.
2. `desk` sends those chunks, still sealed. A move that breaks off resumes from what arrived.
3. `desk` wraps the eidolon key to `tower`'s sealing key and sends it with the next seat record. `tower` checks the record, stores the key and acknowledges.
4. Only then does `desk` delete its key. `desk` can no longer open the eidolon, and `eidolon open` there says `retired`.

`tower` must be on the holder list and must have run `eidolon machine init`. A later move sends only the chunks that changed. If a move is cut off and `desk` can't tell whether `tower` took it, `desk` keeps the eidolon closed. Settle it once you know. `eidolon move-settle <eid> --abandon` takes it back, and `--confirm <seat.json>` accepts the receiver's seat record.

A cell for it has to exist on `tower` behind the same wall, and then `eidolon open <eid> --cell adytum-b` there.

### Trying it on one machine

Two homes on one machine act as two machines. This needs no `ssh`.

```text
EIDOLON_HOME=$HOME/eid-tower eidolon machine init --name tower
EIDOLON_HOME=$HOME/eid-tower eidolon machine show > tower.holder.json
eidolon new --wall wall_work --holder desk.holder.json --holder tower.holder.json --folder notes=/tmp/notes
adytum create adytum-b --sealed --wall wall_work
adytum start adytum-b
eidolon seal <eid> --cell adytum-a
eidolon move <eid> -- env EIDOLON_HOME=$HOME/eid-tower eidolon receive
EIDOLON_HOME=$HOME/eid-tower eidolon open <eid> --cell adytum-b
```

The shell fills in `$HOME` before the command runs. `eidolon verify <eid>` should now pass in `~/eid-tower`, and `eidolon open <eid> --cell adytum-a` on `desk` should be refused.

## The browser facet

An eidolon can hold a Firefox profile. The browser runs in the cell, kept alive by systemd, and its cookies and sign-ins travel with the eidolon.

```text
adytum create adytum-b --sealed --wall wall_work --cpus 2 --memory 3072
adytum start adytum-b
eidolon new --wall wall_work --holder desk.holder.json --browser web
eidolon open <eid> --cell adytum-b
```

The first open of a browser facet sets the cell up. It installs Firefox from Mozilla's package repository, so the cell needs network access and the first open takes longer than later ones. After that the cell has a browser that agents can drive and you can sign in to. You can mix `--folder` and `--browser` in one `eidolon new`.

### Signing in

```text
eidolon view <eid>
```

Eidolon prints an address, `vnc://127.0.0.1:<port>`, and a secret for this session. On a Mac, open the address in Screen Sharing (Finder, Go, Connect to Server) and type the secret as the password. You see the browser, and you sign in as you would on any machine.

- The secret works for this one session and is shown once.
- The window listens on 127.0.0.1 only, and the stream to the cell goes through Adytum's exec channel, so no port of the cell is forwarded.
- While the window is open, the browser runs without its automation socket, so an agent can't drive it. Press Enter in the terminal that runs `eidolon view` when you are done. Automation starts again on the same profile.

### Giving an agent the browser

```text
eidolon mcp <eid>
```

This serves the browser's tools over standard input and output, as an MCP server. Add it to a Claude Code or Codex configuration as a command.

```json
{ "mcpServers": { "eidolon-web": { "command": "eidolon", "args": ["mcp", "<eid>"] } } }
```

The tools are `list_pages`, `new_page`, `select_page`, `close_page`, `navigate_page`, `navigate_history`, `take_snapshot`, `click_by_uid`, `hover_by_uid`, `fill_by_uid`, `type_text`, `press_key`, `get_page_text`, `screenshot_page`, `upload_file_by_uid`, `list_downloads` and `fetch_download`. They name elements by the short uids from a snapshot. There is no tool that runs arbitrary script. The [skill for these tools](../../plugins/xenonet/skills/eidolon-browser/SKILL.md) tells an agent how to use them.

Files pass through one folder on your machine, `~/.eidolon/files/<eid>/`. An agent uploads a file by putting it in that folder and passing its path to `upload_file_by_uid`, and the tool refuses a path that resolves outside the folder. A download is listed with `list_downloads`, and `fetch_download` copies it to the `downloads` folder inside the files folder.

### Serving the tools to other machines

An agent on another machine can't start `eidolon mcp` as a child. Serve the tools over HTTP instead.

```text
eidolon mcp <eid> --http 127.0.0.1:9100
```

The address has to be on this machine. Eidolon makes a token, writes it once to standard error, and serves until you press Ctrl-C. Every request needs `Authorization: Bearer <token>`; a request without it gets 401, and a request that comes from a web page (it carries an `Origin` header) gets 403. Only `/mcp/` answers.

To reach this server from your other machines, publish it as a XenoNet service. This widens what your network can reach, so it asks you (Touch ID on a Mac) and is done in XenoNet's window on the machine that runs `eidolon mcp`.

1. Open **Network settings › Publish a service**.
2. Name it, such as `eidolon-web`.
3. Give its address, `http://127.0.0.1:9100`.
4. Give its path, `/mcp/`.
5. Choose the machines that may call it.

A session on another machine then calls `/api/svc/<machine>/eidolon-web/mcp/` through XenoNet, with XenoNet's credential, and sends the token from `eidolon mcp` as its `Authorization` header, which XenoNet passes on untouched. [A network across your machines](network.md#services-through-the-network) has the rest, including its limits and refusals.

## Security

What Eidolon protects.

- A vault doesn't show what is in it. Each chunk is sealed with ChaCha20-Poly1305 under a key made from the eidolon's key, and named by an HMAC-SHA256 of its content, so two eidolons that share a file don't share a chunk name. A chunk that doesn't open, or opens to another name, is refused.
- There is one live copy. The seat is a signed record with a generation number. A move writes the next generation, signed by the machine that held it, and the sender deletes its key afterwards. A receiver refuses a record that is stale, signed by someone else, or names a machine off the holder list. A move killed at any point leaves one seat.
- The head is checked before it opens. Eidolon opens an eidolon only if a machine on the holder list signed its head and every chunk opens.
- An eidolon stays behind its wall. It refuses to open in a cell behind another wall.
- A move carries sealed chunks and a wrapped key. Whatever carries the stream, `ssh` or XenoNet later, sees nothing it can open.
- The browser is reached over a stream through Adytum's exec channel. No automation port is forwarded, and the sign-in window and the tools are on 127.0.0.1 only.
- A file can't be taken from or put onto your machine by an agent beyond the one files folder. A path that leaves the folder through `..` or a symlink is refused.

What it doesn't protect.

- The keys are in files, not in an operating system key store. Anyone who can read your files as you can read them, and so can open your vaults.
- A site that ties a session to one device, by its fingerprint or its network, asks you to sign in again after a move.
- Eidolon doesn't hide automation. A site can see that a browser is driven, and may ask for a CAPTCHA. Eidolon never tries to get around one.
- A signed-in browser is a signed-in browser. An agent that has the tools can act as that identity on every site it is signed in to, so give the tools only to the sessions for that identity.
- A VM is a strong wall and not a perfect one, and the `adytum` command makes no container cells yet.
- Eidolon moves over a stream you name. Between machines, you are responsible for that stream reaching the right machine. The seat and the holder list still hold if it doesn't.
- There is no Windows or WSL driver yet.

## Related

- [Adytum](adytum.md), which makes the cells.
- [A network across your machines](network.md), which publishes a service.
- [Release notes](../../eidolon/RELEASES.md).
- [The design](../design/cells.md).
