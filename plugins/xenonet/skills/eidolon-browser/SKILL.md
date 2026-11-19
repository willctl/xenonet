---
name: eidolon-browser
description: Drive the browser of an Eidolon through its MCP tools (snapshot, click and fill by uid, uploads and downloads through the files folder). Use when the eidolon MCP tools such as take_snapshot and click_by_uid are available, or when a task needs a signed-in browser kept in an Eidolon.
---

# Driving an eidolon's browser

An eidolon's browser runs in a cell on the person's machine. You drive it through the tools that `eidolon mcp <eid>` serves, over stdio or over XenoNet as a published service. The pages stay signed in as the identity the eidolon holds.

| Tool | What it's for |
| --- | --- |
| `list_pages`, `new_page`, `select_page`, `close_page` | Pages. Every other tool acts on the selected page. |
| `navigate_page`, `navigate_history` | Open an address, or go back and forward. |
| `take_snapshot` | Lists the elements on the page with a uid for each. |
| `click_by_uid`, `hover_by_uid`, `fill_by_uid` | Act on one element from the last snapshot. |
| `type_text`, `press_key` | Type into the focused element, or press a key such as `Control+a`. |
| `get_page_text`, `screenshot_page` | Read the page as text, or as a PNG. |
| `upload_file_by_uid` | Put a file from the files folder into a file input. |
| `list_downloads`, `fetch_download` | List what the browser saved, and copy a finished download out. |

## How to work

1. Call `take_snapshot` first. Act on elements by their uid, never by guessing.
2. A uid works until the page navigates. After any click or navigation that loads a new page, call `take_snapshot` again before the next action.
3. Use `get_page_text` to read, and `screenshot_page` when the layout matters.
4. Files go only through the files folder, `$EIDOLON_HOME/files/<eid>/` (`~/.eidolon/files/<eid>/` by default). To upload, put the file in that folder and call `upload_file_by_uid` with its `filePath`. A path outside the folder, or one that leads out through a symlink, is refused.
5. To download, trigger it in the page, call `list_downloads` until the file is finished (a name ending in `.part` is still being written), then call `fetch_download` with the name. The file lands in the `downloads` folder inside the files folder.

## Never hide the automation

Don't try to make the browser look like a person's. Don't change the user agent, the preferences or the page's scripts to hide that it is driven. There is no tool to run script, and you shouldn't look for another way to run one.

## When to stop and ask

Stop and tell the person when a site asks for a sign-in, a password, a one-time code or a CAPTCHA. Don't try to fill or solve them. The person signs in themselves with `eidolon view <eid>`, which opens a window on the same browser; while it is open, automation is off and the tools fail, and they work again once the person closes the window. Ask them to run it, wait for them to say they are done, then take a new snapshot.

Stop and ask also when a tool says the eidolon is not open, is open elsewhere, or is behind another wall. Those are the person's to fix.
