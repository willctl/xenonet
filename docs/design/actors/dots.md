# dots

Actor `actor:dots`, product id `dots`. **Status: on hold**: the owner has
no ChatGPT Pro yet. Since 2026-10-07 a dot can be paired as a remote actor
that reads, through OpenAI's Secure MCP Tunnel and `xenonet mcp assistant`
([cloud-assistants.md](../cloud-assistants.md)); that's untested with a
real dot, and it doesn't settle anything below about which local threads
are a dot's. Researched on 2026-10-05 for Wave 1
(`w1/actors-research`), read-only. The ChatGPT desktop app is installed on
the development Mac; dots aren't set up.

Claims are marked **official** (OpenAI's docs, the openai/codex source, the
installed app's bundle and shipped code), **secondary** (press), **inferred**
(our reading) or **to confirm** (once a dot is set up).

## In short

- Dots are OpenAI's always-on agents in ChatGPT, launched at DevDay on
  29 Sep 2026 and powered by GPT-6 Astra. Each dot has its own cloud
  computer and browser.
- With access allowed, a dot works on one connected computer through the
  ChatGPT desktop app:
  - it creates local Work or Codex tasks and continues existing local Codex
    tasks;
  - it runs tools, and uses desktop apps and the local browser.
- Of the three cloud actors, XenoNet should see the most of this one. The
  Codex threads a dot creates should come through the `codex app-server`
  XenoNet already reads, and the steps it runs here should run under the
  ChatGPT app's own Codex process. Both are to confirm on the first use.
- What marks a thread as the dot's isn't documented. `threadSource` is the
  likely marker, which XenoNet doesn't read yet; check it on the first use.
- **When it goes ahead:** ChatGPT Pro from $100 a month, and the installed
  ChatGPT app (26.915) has to update to 26.929 or later.

## 1. What it is

| | |
|---|---|
| Maker | OpenAI ([dots docs](https://learn.chatgpt.com/docs/dots)) (official) |
| What | "An always-on agent that keeps work moving across your tools and projects." "Powered by GPT-6 Astra, your dot lives in the cloud and has its own computer and browser." It researches, analyzes data, prepares documents and builds software, and works between conversations. It can split work among background agents and start cloud threads, Codex cloud tasks, and local tasks on a connected computer (official) |
| Status | Launched at DevDay on 29 Sep 2026 (secondary: [TNW](https://thenextweb.com/news/openai-dots-always-on-ai-agents-cloud-computers-devday), [BGR](https://www.bgr.com/2272332/openai-devday-2026-announcements/); OpenAI's own announcement refuses automated reads). "Dots are rolling out gradually to eligible accounts" (official) |
| Plans | **Pro 100, Pro 200 and Pro 500**, for users over 18 outside the EEA, the UK and Switzerland. **Business Premium** and **Enterprise** are rolling out worldwide; Enterprise has dots off until an admin turns them on (official). The Pro tiers cost $100, $200 and $500 a month, and each includes a first dot (secondary). Chatting with a dot doesn't count toward ChatGPT's usage limits; the Work and Codex tasks it starts do (official) |
| Platforms | You create a dot in the ChatGPT desktop app or in ChatGPT in a desktop browser, then reach it in the mobile app, on calls, in Slack and in Microsoft Teams. Texting is "coming soon" (official) |
| Not to be confused with | Codex Remote: the computer connection for Codex from a phone ([remote](https://learn.chatgpt.com/docs/remote)). It's a separate grant: "Your dot's local-computer permission is separate from connecting a computer to Codex or enabling Work Sync" (official) |

## 2. On a Mac: the ChatGPT desktop app

Dots have no app of their own. The local side is the new ChatGPT desktop
app, which is the former Codex app with Chat and Work added (OpenAI's help
article on [moving to the new ChatGPT app](https://help.openai.com/en/articles/20001276)).
These checks were read-only, on the installed app.

| | On the development Mac |
|---|---|
| App | `/Applications/ChatGPT.app`, bundle id `com.openai.codex`, version 26.915.31945 (build 9922). It's arm64 only and needs macOS 13 or later |
| Signed by | Developer ID Application: OpenAI OpCo, LLC (team `2DC432GLL2`), the team XenoNet already checks Codex against |
| Built with | Electron. `Codex Framework.framework` holds the helper apps: `Codex (Renderer)`, `(GPU)`, `(Service)`, `(Alerts)` and their `Aperitif` variants |
| Bundled tools | `Contents/Resources/codex` (codex-cli 0.155.0-alpha.9.2), `codex-code-mode-host`, `rg`, and a Node 24 runtime for computer use (`cua_node`). Native modules, judging by their names, handle input monitoring, the HID topology, remote-control device keys and a launch-services helper |
| Updates | Sparkle, built into the app |
| Launch agents, login items | None in the bundle, and none from OpenAI in `~/Library/LaunchAgents`. Its group containers are `…com.openai.sky.CUAService` (computer use) and `…com.openai.codex.notifications` |
| Running now | No. Only stale crash handlers from older framework versions remain |

**Permissions.** "Your existing ChatGPT app permissions apply to your dot"
([computers and apps](https://learn.chatgpt.com/docs/dots/computers-and-apps)).
So once the computer is connected, whatever the owner granted ChatGPT for
their own use is the dot's too.

| Permission | Asked for | Why it matters with a dot | Advice |
|---|---|---|---|
| Screen Recording and Accessibility | Computer use, through its plugin ([computer use](https://learn.chatgpt.com/docs/computer-use)) | The dot could see the screen and operate any app, including XenoNet's window. It can't automate terminals or ChatGPT itself | Refuse, or revoke before connecting a dot |
| Automation (Apple Events) | "ChatGPT uses Apple Events to control Mac apps on your behalf" (`Info.plist`) | The dot could drive Mail, Messages and Finder | Refuse |
| Calendars, Reminders | Full access, "when you ask" (`Info.plist`) | The dot could read and change both | Refuse unless needed |
| Desktop folder | "For the task you selected" (`Info.plist`) | Files on the Desktop | Per task |
| Microphone, camera, audio capture, location | Voice, video, answers about places (`Info.plist`) | Calls with the dot use the microphone | Microphone only if you call your dot |
| Input Monitoring | The app opens this pane (shipped code) | Global shortcuts; it sees keystrokes | Refuse |
| Locked Use | An authorization plugin that unlocks the Mac for a computer-use turn ([Work local security](https://learn.chatgpt.com/docs/enterprise/chatgpt-work-local-security)) | Unlocks a locked Mac without you there | Never enable |

## 3. State, logs and conversations

**In the cloud** (official: [cloud and local access](https://learn.chatgpt.com/docs/enterprise/cloud-local-access),
[tasks and memory](https://learn.chatgpt.com/docs/dots/tasks-and-memory)):

- The dot's conversations, its own notes (separate from ChatGPT memory), its
  cloud computer with its files and browser sessions, saved logins, and its
  cloud threads.
- Cloud coordination stays involved even when a step runs here: "Local
  execution does not mean offline or device-only model inference."
- There's no strict zero data retention. Enterprise has the Compliance and
  Analytics APIs; a personal plan has nothing comparable.

**On the Mac**, the same files XenoNet already knows from Codex, plus the app's
own:

| Path | What (read for shape only, never values) |
|---|---|
| `~/.codex/state_5.sqlite`, table `threads` | One row per thread, with columns including `source`, `thread_source`, `originator`, `cwd`, `created_at_ms` and `project_id` |
| `~/.codex/state_5.sqlite`, table `remote_control_enrollments` | Probably Codex Remote pairing: `websocket_url`, `account_id`, `app_server_client_name`, `server_id`, `environment_id`, `server_name`, `updated_at`, `remote_control_enabled`. Holds account ids: never shown |
| `~/.codex/sessions/**` | Rollouts. Each one's `session_meta` has `originator`, `source`, `thread_source`, and `creator_user_id` and `creator_account_id` (ids: never shown) |
| `~/Library/Logs/com.openai.codex/YYYY/MM/DD/` | App logs named `codex-desktop-<uuid>-<pid>-t<n>-i<n>-<HHMMSS>-0.log`. Contents not read |
| `~/Library/Application Support/Codex` | The app's Chromium profile: cookies, local storage. Secret |
| `~/Library/Application Support/com.openai.codex` | Updater bootstrap and web assets |
| `~/.codex/auth.json` | Sign-in. XenoNet reads only its ID token claims, never the tokens |

## 4. What XenoNet can observe

### Presence

- **The ChatGPT app running**, with its bundled `codex app-server` beneath
  it. A dot can use the Mac only then ("Keep your computer online and the
  ChatGPT app open while your dot uses it").
- **Connected, Offline or Revoked** shows only in the dot's profile in the
  app, not anywhere XenoNet can read. The dot itself works in the cloud, so
  with the app closed presence is `unknown`.

### The work it does here

**a. Local Codex tasks the dot creates.** The docs say "Local Codex tasks
appear on the connected computer". If they're ordinary Codex threads in
`~/.codex`, as expected, XenoNet lists them with `thread/list` as today. That
they're stored there is to confirm, since "a task can be managed in the
cloud while using files and tools on your connected computer". What would
mark them as the dot's:

| Field | Expected value for a dot's thread | A marker? |
|---|---|---|
| `originator` | `Codex Desktop`, the client name the ChatGPT app uses with its app-server, unless `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` is set (shipped code). Every thread the app creates gets it, the owner's included | No |
| `source` | `vscode`, the app-server's default session source (openai/codex `app-server/src/main.rs`). The owner's threads get it too | No |
| `threadSource` | A label the creating client supplies, an "analytics source classification". The app's shipped code uses `user` by default, and feature labels such as `agent_created_thread` (a thread an agent creates with the app's `create_thread` tool), `agent_forked_thread`, `chatgpt_handoff`, `automation` and `chatgpt_hidden` | **Probably.** The label a dot's tasks get is to confirm: the installed build predates local access for dots |

The protocol for this is official, from the
[app-server schema](https://learn.chatgpt.com/docs/app-server) generated by
the installed codex and the
[openai/codex source](https://github.com/openai/codex). The values come from
the app's shipped code, so they're undocumented and may change.

Two gaps in XenoNet today (`core/codex.js`):

- `threadToSession` ignores `threadSource`. Reading it, and declaring it in
  `vendor/openai/codex/app-server` as a discriminator so drift records keep
  its values, is the first step for this actor. Its values are app labels,
  not user text.
- `thread/list` drops threads whose source is `custom` or `internal`: no
  `sourceKinds` value matches them (`app-server/src/filters.rs`), and
  XenoNet's list doesn't even ask for `unknown`. The app-server on Codex's
  `main` branch has a `--session-source` flag, and `chatgpt` maps to a
  product of its own. If the app ever ran its app-server that way, every
  thread it makes would drop out of XenoNet without a word. A doctor check
  could compare what `thread/list` returns with the size of the `threads`
  table.

**b. Existing tasks the dot continues.** A turn's `turnTrigger` is accepted
by `turn/start` but returned by no read, so a dot's turn in one of the
owner's threads looks like the owner's. XenoNet shouldn't guess. It can
attribute later activity only in threads the dot created.

**c. Steps the dot runs here directly.** Shell commands, file edits, apps
and the local browser, coordinated from the cloud:

- They run through the app's local executor, so they should appear as
  processes under the ChatGPT app's Codex process (inferred; to confirm).
  XenoNet already counts those as Codex, but can't tell them from the
  owner's own Codex work.
- **Codex hooks don't fire for them:** "Command/shell, prompt, and agent
  handlers; hooks from local configuration, plugins, or local directories
  … are not supported with cloud orchestration, even when the task
  executes tools on your computer" (official). XenoNet's Codex hooks
  therefore won't report a dot's local steps. Hooks still fire in threads
  orchestrated and run here; whether a dot's local Codex tasks are of that
  kind is to confirm.

**d. A documented way in: OpenTelemetry.** For dots, "your local executor
can still export supported execution events". Codex exports OTel logs and
metrics when `[otel]` is set in `~/.codex/config.toml`, and every metric
carries `originator` and `session_source` tags
([advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced#observability-and-telemetry)).

- XenoNet could receive them on `127.0.0.1` as a local OTLP endpoint.
- That changes Codex's config, so it's the owner's decision, like
  connecting hooks.
- XenoNet would keep metadata only: tool results carry output snippets, and
  prompts are redacted unless enabled.
- Whether those events tell a dot's steps from the owner's is to confirm.

How to model it, from most to least useful:

1. **Thread markers plus presence** (recommended once a dot is set up):
   `spawned` beams to the threads it creates, `works-on` to their projects.
2. **Add OTel**, if the owner agrees to the config change, for the steps it
   runs here.
3. **Presence only** until then.

### Proposed snapshot

| Field | Value |
|---|---|
| `id`, `kind`, `name` | `actor:dots`, `agent`, "dot" (the owner can rename it; XenoNet doesn't read the dot's own name) |
| `product` | `{ "id": "dots", "vendor": "OpenAI", "version": null }` (a service; the app's version belongs to the app) |
| `via` | `["observed", "attributed"]` |
| `presence` | `working` while one of its threads is active, `idle` while the app runs, `unknown` otherwise |
| `status` | From facts, such as "Started a Codex task in atlas-api", or null |
| `capabilities` | `spawn-sessions`, `edit-repos`, `run-commands`, `browse`, `message-people`, `schedule` |
| `targets` | `codex:<thread id>` (`spawned`), projects (`works-on`) |
| `identity` | The `chatgpt:<id>` identity XenoNet already derives from Codex's sign-in, once the owner confirms the dot uses that account |

## 5. How it acts, and for whom

All official ([controls](https://learn.chatgpt.com/docs/dots/controls),
[messaging](https://learn.chatgpt.com/docs/dots/channels),
[overview](https://learn.chatgpt.com/docs/dots)).

- **For whom:** the owner's ChatGPT account, its plugins' connected
  accounts and their allowed actions. Websites are signed into privately in
  the cloud browser, with optional **Save to Passwords**.
- **Messages:** in ChatGPT, on calls, in Slack and in Teams. "Before sharing
  information from a private conversation with other people, it checks that
  you've allowed it." Its proactive research is read-only: "it cannot send
  messages, change content, or control a browser or computer".
- **Approvals:** automatic review against "your instructions, permissions,
  custom rules, and built-in safety requirements".
  - Custom rules (Settings → Personalization → Permissions) have four
    modes: **Take action without asking**, **Take action when you say so**,
    **Ask before taking action**, **Hand off to you**.
  - Plugin permissions are set separately.
- **Stopping it:**
  - **Activity** shows delegated tasks, local ones included, and **Pause**
    stops the main task.
  - Delegated tasks and **Scheduled** runs are stopped separately.
  - **Delete** can't be undone.

## 6. Sign-in and identity

- **Account:** the ChatGPT account. Local steps need the desktop app
  "running and signed in to the appropriate account and workspace"
  (official).
- **The dot's handle carries a name.** "Your dot starts with a handle such
  as @yourname-dot", and naming the dot changes the handle to
  `@<yourname>-<dot's name>` (official). It's visible wherever the dot
  posts, such as Slack channels.
- **What the app exposes:** the account's name and email (inferred), the
  dot's name and appearance, and the handle.
- **For XenoNet:** the actor can share the `chatgpt:` identity XenoNet already
  has. XenoNet shows that identity's id and email, never a name field, and
  never the dot's handle.

## 7. Install plan (short)

**On hold.** When it goes ahead, every step is the owner's except the
checks:

1. **Plan:** subscribe to ChatGPT Pro 100 or higher ($100 a month,
   secondary), from outside the EEA, UK and Switzerland. Dots may still take
   a while to appear: they're rolling out gradually.
2. **Update the app:** to 26.929 or later, which local access needs
   ([cloud and local access](https://learn.chatgpt.com/docs/enterprise/cloud-local-access)).
   Use the app's own updater. The installed 26.915 is too old.
3. **Create the dot** in the desktop app. Check its handle (section 6)
   before connecting any channel.
4. **Hold back the computer** at first: leave **Computers → Your computer**
   unallowed and try cloud work only.
5. **Review ChatGPT's permissions** in System Settings → Privacy & Security
   before allowing the computer. Revoke Screen Recording, Accessibility,
   Automation, Calendars and Reminders unless the dot should have them, and
   never enable Locked Use.
6. **Allow access** to test: the dot's profile → **Computers** → **Your
   computer** → **Allow access**, then confirm.
7. **Confirm the marker** (an agent, read-only): ask the dot to create a
   local Codex task in a scratch project. Read that thread's `threadSource`,
   `originator` and `source` with `thread/list`, metadata only. Then record
   the process tree of a step it runs here.
8. **Stop:** **Revoke access** in the dot's profile, **Pause** or **Delete**
   the dot, and cancel Pro. The ChatGPT app stays, since Codex runs in it.

Download and size: nothing new to download. The app updates itself in
place; the size of the update isn't published.

## Notes for other lanes

- **Core (vendor schemas):** add `threadSource` (and keep `originator`) to
  `vendor/openai/codex/app-server` and `state-db`, with `threadSource` as a
  discriminator. Add a doctor check for threads `thread/list` can't return.
- **Interface:** if XenoNet ever talks to Codex's shared app-server daemon
  instead of its own private one, be careful. The first client to
  `initialize` a server process sets that process's default originator,
  which threads created afterwards record, and `xenonet` must never become it
  (`initialize_processor.rs`).
- **Security (Wave 3):** a dot with computer use, like Muse, can click in
  XenoNet's window. To XenoNet, that looks like the person, since the window is
  what XenoNet trusts for "yours alone" actions.

## The Phase 0 notes, checked

| Phase 0 said | Now |
|---|---|
| Announced at DevDay on 29 Sep 2026 (unverified) | The press says so. OpenAI's own announcement and help pages refuse automated reads, so this stays secondary |
| Runs on GPT-6 Astra (unverified) | **Verified** in OpenAI's docs |
| Plans: Pro 100, 200 and 500, Business Premium, Enterprise | **Verified.** The prices are secondary |
| The new ChatGPT app is the old Codex app with Chat and Work | **Verified:** the installed app's bundle id is `com.openai.codex`, and OpenAI's help article covers the move |
| Which `source` or `originator` marks a dot's threads (unverified) | **Neither should:** every thread the app makes is expected to have `originator` `Codex Desktop` and `source` `vscode`. `threadSource` probably does; its value is to confirm |
| Turn it on in the dot's profile: Computers → Your computer → Allow access | **Verified.** **New:** it needs app 26.929 or later (26.915 is installed), and Codex hooks don't fire for the steps it runs here |

## Open questions

- When will ChatGPT Pro be bought, and which tier? It decides whether this
  actor is built in Wave 2 or later.
- Should the dot get the Mac at all, or stay cloud-only, with XenoNet showing
  only the Codex threads it starts?
- Set up OTel export to XenoNet? It's a change to Codex's config.

## Sources

Official:

- learn.chatgpt.com: [Meet dots](https://learn.chatgpt.com/docs/dots),
  [computers and apps](https://learn.chatgpt.com/docs/dots/computers-and-apps),
  [tasks and memory](https://learn.chatgpt.com/docs/dots/tasks-and-memory),
  [controls](https://learn.chatgpt.com/docs/dots/controls),
  [messaging](https://learn.chatgpt.com/docs/dots/channels),
  [getting started](https://learn.chatgpt.com/docs/dots/getting-started),
  [admin guide](https://learn.chatgpt.com/docs/enterprise/dots-admin-guide),
  [local computer access for Work Cloud and dots](https://learn.chatgpt.com/docs/enterprise/cloud-local-access),
  [Work local security](https://learn.chatgpt.com/docs/enterprise/chatgpt-work-local-security),
  [computer use](https://learn.chatgpt.com/docs/computer-use),
  [app server](https://learn.chatgpt.com/docs/app-server),
  [advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced),
  [Codex Remote](https://learn.chatgpt.com/docs/remote).
- [Moving to the new ChatGPT desktop app](https://help.openai.com/en/articles/20001276)
  (refuses automated reads; known from its title and search summaries).
- [openai/codex](https://github.com/openai/codex), `main` on 2026-10-05:
  `codex-rs/protocol/src/protocol.rs` (`SessionSource`, `ThreadSource`,
  `SessionMeta`), `codex-rs/app-server/src/filters.rs`,
  `codex-rs/app-server/src/main.rs`,
  `codex-rs/app-server/src/request_processors/initialize_processor.rs`.
- The installed app: `Info.plist`, `codesign`, the bundle's layout, the
  schema from `codex app-server generate-json-schema`, and string searches
  of its shipped code. The SQLite files were read for tables and columns
  only.

Secondary:

- [TNW](https://thenextweb.com/news/openai-dots-always-on-ai-agents-cloud-computers-devday)
  and [BGR](https://www.bgr.com/2272332/openai-devday-2026-announcements/):
  the DevDay launch.
- Pro prices: press and plan comparisons.
