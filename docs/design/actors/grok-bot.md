# Grok Bot

Actor `actor:grok-bot`, product id `grok-bot`. **Status: goes ahead**: the
owner has a SuperGrok plan. Researched on 2026-10-05 for Wave 1
(`w1/actors-research`), read-only. Grok Bot isn't installed on the
development Mac, so everything local below comes from the vendor's docs, its
update feed and the Homebrew cask, and is marked for checking once it's
installed. XenoNet's observer of it was built in Wave 2 and settled in Wave 3:
[As built](#as-built) says what it does.

Each claim that matters says where it comes from:

- **official**: SpaceXAI's or Cursor's docs and posts, the vendor's update
  feed, the App Store;
- **secondary**: press, the Homebrew cask, release trackers;
- **inferred**: our reading of the evidence;
- **to confirm**: to check on the Mac after the install (Wave 2).

## In short

- Grok Bot is SpaceXAI's always-on agent product, built and run by Cursor
  (Anysphere, part of SpaceX since 14 Aug 2026). Bots work on one shared
  cloud computer per member and keep going with the app closed.
- On the Mac, the desktop app is the Bots' only way in. It can run commands,
  read files and move files, each command behind an approval card by
  default, and it can route the cloud computer's web traffic through the Mac.
- XenoNet can see that the app is running and, by process ancestry, the
  commands Bots run on the Mac. Nothing else a Bot does is visible locally,
  and an individual plan has no API for it.
- The install is a 145 MB DMG (0.66.0, Apple silicon) and a Cursor account
  (a free one is enough) with the owner's SuperGrok linked to it. **The link
  is permanent.** Keep local execution on **Never allow** or **Ask every
  time** and egress routing off, and give the app no Full Disk Access,
  Accessibility, Screen Recording or Automation.

## As built

The observer is `core/actors/grok-bot.js`, built in Wave 2 on the plan
below (presence and process ancestry, recommended in section 4) and
settled by the owner's calls at merge point B and in Wave 3.

- **Where it looks.** `Grok Bot.app` in `/Applications` or
  `~/Applications`. Not installed there and never seen, the process table
  is read once a minute, for an app run from somewhere else.
- **Presence.** `working` while a command its app runs is running, or ran
  in the last 30 s; `idle` while the app runs ("Its app is open on this
  Mac"); `unknown` with the app closed ("Its app on this Mac is closed"),
  since Bots go on in the cloud and nothing here can say they stopped.
  It's never `offline`, however long the app has been closed, and it stays
  listed, out of sight, for as long as the app is installed (the owner's
  call in Wave 3; other installed products are listed as offline once
  they stop running). `heartbeatMs` is null.
- **Commands.** A process the app starts whose program lies outside its
  bundle is a command: a `ran` event, gathered per minute in the project
  holding the folder it started in, by count and place, never by its
  arguments. One that is `claude` or `codex`, or starts one, started a
  session (`spawned`), matched to the session as `attribution.js` does.
  While working, its status is "Running commands in atlas-api".
- **How often.** The observers read the whole process table together
  every 5 s. While the app runs, Grok Bot's also looks at what the app
  runs every 3 s with `pgrep` (the owner's call at merge point B, against
  the once a second section 4 asked for), and describes only the pids
  that are new: a few milliseconds of CPU. A command that starts and ends
  between two looks is missed.
- **Its version** is `CFBundleShortVersionString` from the app's
  `Info.plist` (the owner's call at merge point B): read with a regular
  expression from an XML plist, or with `plutil -extract` from a binary
  one, again only when the file changes, and shown only when it reads as a
  version.
- **Its bundle id**, `CFBundleIdentifier`, is read beside it since
  2026-10-08, as Muse's is: a `Grok Bot.app` is Grok Bot's only when the id
  is Cursor's `com.anysphere.sand`, in the observer and in
  `core/procs.js`, since anyone can name a bundle that. Those two keys are
  all it reads of the `Info.plist`.
- **Nothing else** of the app's files is read: not its data, logs,
  preferences or tokens. Its observer's switch, in the Actors panel or
  `xenonet actor observe grok-bot off`, takes it off the map.
- **Reporting itself (2026-10-08, the owner's call, reversing merge point
  A).** A Bot can register `actor:grok-bot` by running `xenonet actor
  register --product grok-bot` through local execution, each run approved
  in the app, and the owner's Allow. It joins the observed actor; its token
  lives in a 0600 file on the Mac, not in a Bot's instructions. The token
  works only from a command under Grok Bot's app as Cursor signed it (team
  `DCNK4UB866`, checked with `codesign` on the app process nearest the
  caller), never from the app's own processes or a bundle that only calls
  itself Grok Bot.app. With `message-agents` it messages other actors and
  agents, such as Muse, and reads its inbox. Once registered, its `ran`
  events are left to its own reports. Any command under the app, the Bot's
  own included, can read the token file; that's the trust this gives a
  Bot, and why each command still needs approval.
- **Or the gateway.** A Bot can also reach XenoNet from Cursor's cloud,
  through XenoNet's actor gateway (`server/actor-gateway.js`), added as an
  MCP server by URL with a gateway token as its Authorization header. That
  token works only at the gateway; messages through it are marked as such.
- **The doctor** says whether the app is installed and whether XenoNet
  could read the processes, the app and its version.

## 1. What it is

| | |
|---|---|
| Maker | SpaceXAI (formerly xAI), with Cursor. Announced on 11 Aug 2026 on [x.ai](https://x.ai/news/introducing-grok-bot). The docs live on both [docs.x.ai](https://docs.x.ai/grok-bot) and [cursor.com](https://cursor.com/docs/grok-bot), and the service runs on Cursor's cloud. Cursor became part of SpaceX on 14 Aug 2026 ([Cursor's post](https://cursor.com/blog/joining-spacex)). (official) |
| What | "AI teammates you can give real work to". Each Bot has a name, one primary job and a memory. All of a member's Bots share one persistent cloud computer (a Firecracker microVM) with a browser that keeps logins, a `/workspace` folder and a terminal. Bots message each other, work in group chats of 2 to 6 Bots, run routines on schedules or events, and stop for approval before consequential actions. (official) |
| Status | In beta ([announcement](https://x.ai/news/introducing-grok-bot)). Desktop 0.66.0 on the stable feed, its DMG dated 1 Oct 2026 (official feed); pre-releases have reached 0.68.0-pre (secondary, [a release tracker](https://techdevnotes.com/releases/grok-bot-desktop-extra)). iOS app 1.13.1, 1 Oct 2026 ([App Store](https://apps.apple.com/us/app/grok-bot/id6794501026)). |
| Platforms | Desktop: macOS (Apple silicon and Intel), Windows (x64, Arm64), Linux (x64, Arm64; .deb, .rpm, AppImage). Mobile: iPhone and iPad on iOS 18 or later, Android 9 or later. ([get started](https://docs.x.ai/grok-bot/get-started), [mobile](https://docs.x.ai/grok-bot/mobile), [deployment](https://cursor.com/docs/grok-bot/deployment)) (official) |
| Plans | Paid individual Cursor plans (Pro, Pro+, Ultra), Cursor Teams (Standard, Premium) and Enterprise. Or an individual SuperGrok, SuperGrok Plus, SuperGrok Heavy or X Premium+ linked to a Cursor account; SuperGrok Lite, Team and Enterprise can't link. The free Cursor plan alone doesn't include Grok Bot. Plans don't stack. The trial is a usage credit with a 7-day window. ([plans](https://cursor.com/help/grok-bot/plans), [FAQ](https://cursor.com/help/grok-bot/faqs), [pricing](https://cursor.com/pricing)) (official) |
| Prices | Cursor Individual from $20 a month, Teams $40 per user a month ([pricing](https://cursor.com/pricing)); bought in the iOS app, Pro $25.99, Pro+ $77.99 and Ultra $259.99 (App Store). SuperGrok Lite $10, SuperGrok $30 and Heavy $300 a month, with Plus reported at $100 (secondary only: x.ai's plan pages refuse automated reads). **For the owner: nothing more**, if their SuperGrok tier can link. |
| Regions | No regional limits are documented. The cloud computers run in the US ([security FAQ](https://cursor.com/docs/grok-bot/security-faq)). The App Store rates it 18+. (official) |
| Not to be confused with | Grok Build, the `grok` coding CLI ([x.ai/build](https://x.ai/build)), and the Grok chat app (iOS bundle `ai.x.GrokApp`). Neither is on this Mac. |

## 2. On a Mac

### The app

- **Install:** a DMG with `Grok Bot.app`, dragged to `/Applications`. The
  Homebrew cask `grok-bot` installs the same file (official; secondary).
- **Bundle id:** probably `com.anysphere.sand`. The Homebrew cask's cleanup
  paths use it, the iOS app is `co.anysphere.sand`, and Cursor's update feed
  calls the product `sand` (inferred; to confirm in `Info.plist`).
- **Minimum macOS:** not documented; the cask says macOS 12 or later
  (secondary).
- **Updates:** the app checks for updates itself and offers each new
  version. Old builds stop working behind an **Update required** screen
  ([deployment](https://cursor.com/docs/grok-bot/deployment)) (official).
- **Built with:** probably Electron, judging by the silent NSIS installer on
  Windows (`/S`), the AppImage/.deb/.rpm packages, the 145 MB DMG and a data
  folder named after the product (inferred; to confirm).
- **Helper processes:** not documented. Expect Electron helpers named after
  the app (to confirm with `ps`).
- **Launch agents and login items:** none documented. The cask removes no
  launch agent when it uninstalls (secondary; to confirm with
  `launchctl print gui/$(id -u)`, `~/Library/LaunchAgents` and System
  Settings → General → Login Items).
- **Network:** installers and updates come from `cursor.com` and
  `downloads.cursor.com`. The app talks to Cursor's API and to the member's
  cloud computer on `*.cursorvm.com` patterns
  ([deployment](https://cursor.com/docs/grok-bot/deployment)) (official).
- **In the background:** "Closing the app, the laptop, or the phone does not
  stop a task or a routine." To quit fully, choose **Quit**
  ([FAQ](https://cursor.com/help/grok-bot/faqs)) (official).

### macOS permissions

The docs mention only notifications and the microphone. The rest follows
from the app running commands for Bots: macOS charges a command's access to
the app that started it, so **whatever Grok Bot is granted, every Bot's
local command gets**.

| Permission | Asked? | Why it matters for privacy | Advice |
|---|---|---|---|
| Notifications | Yes, when a Bot's **Notifications** switch is on ([settings](https://docs.x.ai/grok-bot/settings-and-notifications)) (official) | Banners show Bot names and results on screen | Allow if wanted |
| Microphone | Yes, for dictation (`Cmd+D`) and voice chat ([voice chat](https://cursor.com/help/grok-bot/voice-chat)) (official) | Audio goes to the service | Refuse unless you use voice |
| Files and Folders (Desktop, Documents, Downloads, iCloud Drive, removable volumes) | Expected the first time a Bot's command touches one (inferred) | A grant covers every later command from every Bot | Refuse by default; allow one folder when a task needs it |
| Full Disk Access | Not documented | Every command any Bot runs would read Mail, Messages, browser history and other apps' data. Apple said on 2 Oct 2026 it will make this grant harder because of agents ([Apple](https://developer.apple.com/news/?id=p6zjojqw)) | Never grant |
| Accessibility, Screen Recording, Input Monitoring | Not documented: its local access is commands and files. Driving apps takes a third-party driver run as a command ([one such guide](https://cua.ai/docs/use-cua-with/grok-bot), secondary) | A command could watch or drive the whole Mac | Never grant |
| Automation (Apple Events) | Not documented; a command such as `osascript` would bring a per-app prompt naming Grok Bot (inferred) | Commands could control Mail, Messages or Finder | Refuse |
| Local Network | Possibly, with egress routing on (inferred) | Cloud Bots could reach the LAN, and maybe services on this Mac | Refuse, and keep routing off |
| Security keys | **Use hardware security keys** (Settings → General → Security Key) lets the cloud browser use a key plugged into the Mac. On by default on macOS, and every use asks for approval ([approvals](https://docs.x.ai/grok-bot/approvals-security-and-privacy)) (official) | A Bot could sign in with your key | Turn it off unless you sign in with a key |

### The settings that matter

All official, from [approvals, security and privacy](https://docs.x.ai/grok-bot/approvals-security-and-privacy),
[settings](https://docs.x.ai/grok-bot/settings-and-notifications) and
[security](https://cursor.com/docs/grok-bot/security).

- **Execution on Local Computer** (Settings → General → Bot). Choices: **Ask
  every time** (the default), **Always allow**, **Never allow**.
  - Once the account has registered computers, the choice moves to Settings
    → Computer → Computers, as **Execution on this computer** for each one.
  - The first request shows "Allow Grok Bot and all Bots to run commands on
    your local computer?" with **Always allow**, **Allow once**, **Never**
    and **Deny once**. **Always allow** and **Never** set the policy for
    every Bot.
  - The approval card shows the exact command. The docs recommend **Never
    allow** unless a Bot has a specific reason to work on local files.
- **Route egress through this desktop** (Settings → Computer). It sends the
  cloud computer's web traffic through the Mac: "Destinations see your
  desktop's IP address, and the Bot can reach networks available from that
  device." Keep it off.
- **Auto-review** (Settings → General): personal rules, **Ask first** or
  **Allow automatically**, where **Ask first** wins. They're "stored on the
  current desktop and synced to its Grok Bot computer".

## 3. State, logs and conversations

**In the cloud** (official: [security](https://cursor.com/docs/grok-bot/security),
[approvals](https://docs.x.ai/grok-bot/approvals-security-and-privacy),
[routines](https://docs.x.ai/grok-bot/skills-routines-and-automations)):

- Bots, conversations, memory, and routines with their last 20 runs each.
- The computer's durable disk: `/workspace`, browser sessions and saved
  logins, and command-line credentials, shared by all of a member's Bots.
- Connector tokens stay on Cursor's backend, never on the computer.
- Daily encrypted backups. Data is deleted within 30 days of the service
  ending, under the DPA.
- Grok Bot needs data storage and doesn't support Legacy Privacy Mode.
  Training opt-out follows the Cursor account's privacy settings.
- Enterprise only: audit logs, **Action Recording** (scrubbed shell
  commands, kept 90 days) and OpenTelemetry export. An individual or
  SuperGrok-linked account doesn't get them.

**On the Mac**, the vendor documents only that personal Auto-review rules are
stored on the desktop. The Homebrew cask's cleanup list names the folders
(secondary):

| Path | Probably |
|---|---|
| `~/Library/Application Support/Grok Bot` | The app's data folder: local storage, caches, maybe cached conversations and the session |
| `~/Library/Application Support/com.anysphere.sand` | Updater or native state |
| `~/Library/Caches/com.anysphere.sand` | Caches |
| `~/Library/HTTPStorages/com.anysphere.sand` | Cookies of the app's native networking |
| `~/Library/Logs/Grok Bot` | The app's logs |
| `~/Library/Preferences/com.anysphere.sand.plist` | Preferences |
| `~/Library/Saved Application State/com.anysphere.sand.savedState` | Window state |

Treat all of these as secret until checked: the sign-in session, and possibly
cached messages and commands, live there. XenoNet reads none of them. In Wave
2, with the owner's OK, an agent may list the file names and the shapes of
log lines (keys and types, never values), to see whether a line marks "local
computer connected" or "command approved" without any text.

## 4. What XenoNet can observe

### Presence

- **The app running:** a process whose program is inside `/Applications/Grok
  Bot.app/Contents/MacOS/` (the name is to confirm). XenoNet's process table
  (`core/procs.js`) already lists it. Its signature's team id is to confirm
  (Anysphere's, or a SpaceX one), so XenoNet can check it the way it checks
  Claude Code's and Codex's.
- **What that means:** the app running means the Mac can take local work. It
  doesn't mean a Bot is working. With the app closed, Bots still work in the
  cloud and XenoNet can't tell, so presence is `unknown` then, never `offline`.
- **Local API or socket:** none documented. Check after the install with
  `lsof -nP -a -p <pid> -i -U`, for the record only: XenoNet shouldn't use a
  private one.

### The work it does here

The commands Bots run come from the desktop app. Expected process tree, to
confirm:

```
launchd
└─ Grok Bot                         /Applications/Grok Bot.app/Contents/MacOS/…
   ├─ Grok Bot Helper (Renderer)    Electron helpers (names to confirm)
   ├─ Grok Bot Helper (GPU)
   └─ main process or a helper
      └─ /bin/zsh -c '<command>'    the shell it uses (login shell? sandbox?)
         └─ git status              what the Bot asked for, once approved
```

- **Attribution:** a process whose ancestry reaches the Grok Bot main
  process, and whose program lies outside the app's own bundle (so not one
  of its helpers), is the Bot's (`procs.chain()`). XenoNet records a `ran`
  event whose target is the project holding the process's working folder
  (`lsof -a -p <pid> -d cwd`). The detail is the program's name and the
  project, never its arguments, which can hold tokens or file contents.
- **Sessions it starts:** when the command starts `claude` or `codex`, the
  session's process sits under Grok Bot, so the actor gets a `spawned` beam
  to it. That's the strongest link XenoNet can draw.
- **Short commands:** XenoNet's process table is shared for 1.5 s and taken
  only when asked. While the app runs, polling its descendants about once a
  second catches most commands. Commands shorter than that are missed; the
  actor still shows it ran commands in a project. (As built, the poll is
  every 3 s: see [As built](#as-built).)
- **Files moved between the cloud computer and the Mac:** where they land
  isn't documented. XenoNet sees them only as changes in the git status of
  repos under the root, without knowing who made them.
- **Egress routing and approval cards:** neither is visible without watching
  the network or the app's window. XenoNet shouldn't try, so a question
  waiting for the owner never shows on this actor.

### What can't be seen here

Everything in the cloud: browsing, drafts of email and Slack messages,
routines, Bot-to-Bot messages, conversation text. Also which Bot ran a local
command: all Bots share the computer and the one local permission. No public
API exposes a member's Bots or their activity, and the Enterprise audit
features aren't available on this plan.

How to model it, from most to least useful:

1. **Presence and process ancestry** (recommended). It changes nothing on
   the Mac or in Grok Bot.
2. **Self-report**, later and optional. A Bot can run `xenonet actor …`
   through local execution, each run approved, to say which Bot it is and
   what it's doing. Low trust: XenoNet can only check that the call came from
   under Grok Bot.
3. **Presence only**, if the owner keeps local execution on **Never allow**.

### Proposed snapshot

| Field | Value |
|---|---|
| `id`, `kind`, `name` | `actor:grok-bot`, `agent`, "Grok Bot" (the owner can rename it) |
| `product` | `{ "id": "grok-bot", "vendor": "SpaceXAI", "version": <CFBundleShortVersionString> }` |
| `via` | `["observed", "attributed"]` |
| `presence` | `working` while a command runs under the app (a descendant from outside its bundle), `idle` while only the app runs, `unknown` when the app isn't running |
| `heartbeatMs` | null (no heartbeat) |
| `status` | From facts, such as "Ran 3 commands in atlas-api", or null |
| `capabilities` | `run-commands`, `browse`, `message-people`, `message-agents`, `schedule` |
| `targets` | Projects (`works-on`), sessions it started (`spawned`) |
| `identity` | null (section 6) |

## 5. How it acts, and for whom

All official ([security](https://cursor.com/docs/grok-bot/security),
[approvals](https://docs.x.ai/grok-bot/approvals-security-and-privacy),
[messaging](https://docs.x.ai/grok-bot/chat-and-collaboration)).

- **For whom:** "A Bot has no identity or credentials of its own." Bots act
  as the signed-in member, can't hold more access than the member, and
  "every action stays attributable to a named member". Connectors use the
  member's accounts through tokens kept on Cursor's backend.
- **Messages:** drafts of emails and Slack messages appear for review, and
  the member chooses **Send email** or **Send message**. Bots message each
  other and work in group chats; they also notify the member and come back
  with results and questions.
- **Approvals:** the docs advise explicit boundaries for sending messages or
  invitations, publishing, purchases and transfers, deleting or overwriting
  data, changing permissions, production changes and accepting legal terms.
  - The choices are **Allow once**, **Always allow** or **Deny**; Auto-review
    rules shape which actions stop.
  - "An approval controls the proposed action. It does not reverse work
    already completed."
- **Passwords, passkeys, two-factor codes, CAPTCHAs and payment
  confirmations:** the Bot hands the cloud computer to the member (**Agent
  Computer**) instead of typing them.
- **On the Mac:** one setting for all Bots, per-command approval by default.
  Approvals also work from the phone.

## 6. Sign-in and identity

- **Account:** "Grok Bot uses your Cursor account. There is no separate Grok
  Bot login." The desktop app finishes sign-in in the browser; teams can use
  SSO ([sign in](https://cursor.com/help/grok-bot/sign-in)) (official).
- **SuperGrok:** on the plan screen, **Link Grok Account**, sign in with the
  Grok account that holds an individual SuperGrok, then confirm (**Link X
  Account** for X Premium+). With no Cursor plan, usage comes from the
  SuperGrok grant alone ([SuperGrok](https://cursor.com/help/grok-bot/supergrok),
  [plans](https://cursor.com/help/grok-bot/plans)) (official).
- **The link is permanent.** "You can't unlink a Grok or X account from a
  Cursor account, and you can't move a link to a different Cursor account."
  One Cursor account to one Grok or X account, and support can't undo it
  (official).
- **What the app shows** (to confirm after the install):
  - the Cursor account under Settings → Account, with its email and the
    profile's name and avatar;
  - the Bots' names and avatars, chosen by the member;
  - each registered computer under Settings → Computer → Computers. Its name
    is probably the Mac's own name, which often holds a person's name.

  The App Store's privacy label lists name and email as data linked to the
  user.
- **For XenoNet:** `identity` stays null. XenoNet could learn the Cursor account
  only from the app's storage, which it won't read. It never shows the
  account's email or name; the owner can name the actor.

## 7. Install plan

### Decisions for the owner first

1. Which Cursor account Grok Bot uses (a new one, or an existing one), and
   which Grok account holds the SuperGrok plan. The link is permanent and
   one to one.
2. That the plan is SuperGrok, SuperGrok Plus or SuperGrok Heavy (Lite can't
   link). Otherwise Grok Bot needs a paid Cursor plan, from $20 a month.
3. The local execution policy for the trial: **Never allow** (presence only),
   or **Ask every time** to test XenoNet's attribution with a harmless command.

### Download (an approved download in Wave 2)

- **Source** (official): the docs point to the
  [downloads page](https://x.ai/bot), which refuses automated reads. The
  macOS link in the [announcement](https://x.ai/news/introducing-grok-bot),
  `https://api2.cursor.sh/updates/download/stable/darwin-arm64/grok-bot-bd824e1890d8b96f`,
  redirects to
  `https://downloads.cursor.com/grokbot/stable/darwin-arm64/0.66.0/Grok_Bot_0.66.0.dmg`.
  The feed `https://api2.cursor.sh/updates/api/download/stable/darwin-arm64/sand`
  names the current version.
- **Size** (HEAD requests on 2026-10-05): 144,945,109 bytes (145 MB) for
  Apple silicon; the Intel build is 154,445,169 bytes. Both dated 1 Oct
  2026.
- **SHA-256** (from the Homebrew cask, secondary): Apple silicon
  `3da5e4a564b8f4292a6fcbbe651bed3a01630d01f959232b037b9f0acbdf5de6`; Intel
  `7198b0b0bf40f4e23c9c8086a871bb25b90ee1babc03ad3e322e0f872645f1fd`. If the
  feed has moved on by Wave 2, take the version it names and that version's
  cask hash.
- **Requirements:** an Apple silicon Mac. The development Mac is arm64 on
  macOS 26.5.2, with 64 GB of memory and about 78 GB free. Also a Cursor
  account and a SuperGrok that can link.
- **Cost:** nothing more, with an eligible SuperGrok.

### Steps

| # | Step | Who |
|---|---|---|
| 1 | Download the DMG | An agent with the owner's approval, or the owner |
| 2 | Check it: `shasum -a 256`, then on the mounted image `codesign -dv --verbose=4` and `spctl -a -vv -t exec` on the app. Note the team id, bundle id and version | Agent, read-only |
| 3 | Drag `Grok Bot.app` to `/Applications` and open it; accept Gatekeeper's "downloaded from the internet" prompt | **Owner** |
| 4 | Sign in with the Cursor account in the browser | **Owner only** |
| 5 | Plan screen → **Link Grok Account** → sign in to the Grok account → confirm (permanent) | **Owner only** |
| 6 | In onboarding, skip connecting tools (mail, Slack, calendar) for now | **Owner** |
| 7 | Settings → General → Bot → **Execution on Local Computer**: **Never allow**, or **Ask every time** for the test in step 12 | **Owner** |
| 8 | Settings → Computer: **Route egress through this desktop** off; check the registered computer's name | **Owner** |
| 9 | Settings → General → Auto-review: add "Ask first before sending any external email or message" | **Owner** |
| 10 | Refuse macOS prompts for Full Disk Access, Accessibility, Screen Recording, Input Monitoring, Automation and Local Network; allow the microphone only for voice, and notifications if wanted. Turn off **Use hardware security keys** unless you sign in with a key | **Owner** |
| 11 | Record for Wave 2, read-only: `ps -axo pid,ppid,comm` with the app open, `lsof` sockets of its processes, the file names in the folders of section 3, `launchctl print gui/$(id -u)` and Login Items | Agent |
| 12 | Optional, with **Ask every time**: ask a Bot to run `pwd` in a scratch project, choose **Allow once**, and check that XenoNet sees the process under Grok Bot | Owner, then an agent |

### Uninstall

1. Set **Execution on Local Computer** to **Never allow** and turn off egress
   routing, so a reinstall doesn't bring access back by itself.
2. Quit Grok Bot (**Quit** from the menu).
3. Move `/Applications/Grok Bot.app` and the folders in section 3 to the
   Trash. The owner empties it.
4. In System Settings → Privacy & Security, remove whatever was granted
   (notifications, microphone, files and folders).
5. Uninstalling only signs out: the account, the Bots and the cloud computer
   stay. To clean the cloud side, delete routines, Bots and connectors, sign
   out of websites and clear `/workspace`
   ([approvals](https://docs.x.ai/grok-bot/approvals-security-and-privacy)).
6. To delete everything, cancel subscriptions and delete the Cursor account
   at cursor.com/dashboard; Grok Bot data goes within 30 days
   ([delete account](https://cursor.com/help/grok-bot/delete-account)). The
   SuperGrok subscription is separate, and the link can't be undone.

## 8. After the install (2026-10-08)

Recorded read-only on the development Mac, with the app open and signed
in: names and shapes only. Of the app, only its `Info.plist`, its
signature and the strings in its bundled code were read; nothing under its
data folders was opened.

| | |
|---|---|
| App | `/Applications/Grok Bot.app`, version 0.68.1, bundle id `com.anysphere.sand` (as section 2 guessed), team id `DCNK4UB866`. Apple silicon only; `LSMinimumSystemVersion` 12.0 |
| Built with | Electron, updated by Squirrel. It asks Cursor's update server for its product, `sand`: `https://api2.cursor.sh/updates/api/download/stable/darwin-arm64/sand` answers JSON with the version (0.68.1 on 2026-10-08). The provider drift check reads it (`spacexai/grok-bot-desktop`) |
| Processes | `Contents/MacOS/Grok Bot` under launchd, its Electron helpers (`Grok Bot Helper`, `(Renderer)`, `(GPU)`) under it, and `chrome_crashpad_handler` under launchd. No commands ran under it while it sat open |
| Network | One TLS connection out, and Electron's own single-instance socket. No local port that another program could talk to |
| Folders | `~/Library/Application Support/Grok Bot` (Electron's), `Caches/com.anysphere.sand` and `com.anysphere.sand.ShipIt` (the updater's), `HTTPStorages/com.anysphere.sand`, and `Preferences/com.anysphere.sand.plist`. No log folder under `~/Library/Logs` |
| MCP | The app manages MCP servers for Bots: remote ones by URL, with OAuth, and ones run by a command, which run "on Grok Bot's computer", the cloud one. So a Bot's MCP servers can't reach XenoNet on this Mac; its local commands can |

## Notes for other lanes

- **Security (Wave 3).** With egress routing on, cloud Bots "can reach
  networks available from that device", which may include `127.0.0.1`,
  where XenoNet's API listens. Such requests would come from the Grok Bot
  process. XenoNet's process check refuses them for calls in an agent's name,
  but read-only routes answer any local process today. Worth a look, and a
  reason to keep routing off.
- **Interface.** For cloud agents, local evidence can't prove `offline`:
  presence should fall back to `unknown`, with the desktop link (app up or
  down) in `status`.
- **Process polling.** Catching commands needs a descendant poll of about
  1 Hz while the app runs. The benchmark should cover it. (Built at every
  3 s, the owner's call at merge point B, and the benchmark's looks read
  the app's version as the real ones do.)

## The Phase 0 notes, checked

| Phase 0 said | Now |
|---|---|
| SpaceXAI, formerly xAI | Yes, and Cursor builds and runs it: the docs live on cursor.com too, and the computers are Cursor's. SpaceX acquired Cursor's maker on 14 Aug 2026 |
| Also Windows, Linux and Android, from x.ai/bot (unverified) | **Verified:** macOS, Windows, Linux, iOS and Android, with downloads from x.ai/bot |
| Sign in with a Cursor account and link SuperGrok; how does a SuperGrok-only user sign in? (unverified) | **Verified:** a Cursor account is always needed (a free one will do), with the SuperGrok linked to it. **New:** the link is permanent and one to one, Lite can't link, and X Premium+ can |
| Settings → General → Bot → Execution on Local Computer, three choices, the card shows the command | **Verified.** Once the account has registered computers, the setting moves to Settings → Computer → Computers, as **Execution on this computer** for each |
| No local logs, data or API documented | Still true of the vendor's docs. The Homebrew cask names the data folders (section 3) |

## Open questions

- Which Cursor account, and does its profile carry a real name?
- Which SuperGrok tier?
- Allow local execution in the trial at all, or keep Grok Bot presence-only?
- After the install: answered in section 8, except the shell commands run
  in, the logs' shapes, the registered computer's name, and whether the app
  adds a login item.

## Sources

Official:

- [Introducing Grok Bot](https://x.ai/news/introducing-grok-bot), SpaceXAI,
  11 Aug 2026.
- docs.x.ai: [overview](https://docs.x.ai/grok-bot),
  [get started](https://docs.x.ai/grok-bot/get-started),
  [use the computer and apps](https://docs.x.ai/grok-bot/computer-and-apps),
  [approvals, security and privacy](https://docs.x.ai/grok-bot/approvals-security-and-privacy),
  [settings and notifications](https://docs.x.ai/grok-bot/settings-and-notifications),
  [message and collaborate](https://docs.x.ai/grok-bot/chat-and-collaboration),
  [skills and routines](https://docs.x.ai/grok-bot/skills-routines-and-automations),
  [mobile](https://docs.x.ai/grok-bot/mobile),
  [private networks](https://docs.x.ai/grok-bot/private-networks),
  [manage computers](https://docs.x.ai/grok-bot/computers),
  [troubleshooting](https://docs.x.ai/grok-bot/troubleshooting),
  [FAQ](https://docs.x.ai/grok-bot/faq).
- cursor.com: [security](https://cursor.com/docs/grok-bot/security),
  [security FAQ](https://cursor.com/docs/grok-bot/security-faq),
  [deployment](https://cursor.com/docs/grok-bot/deployment),
  [settings](https://cursor.com/docs/grok-bot/settings),
  [plans](https://cursor.com/help/grok-bot/plans),
  [SuperGrok](https://cursor.com/help/grok-bot/supergrok),
  [sign in](https://cursor.com/help/grok-bot/sign-in),
  [FAQ](https://cursor.com/help/grok-bot/faqs),
  [delete account](https://cursor.com/help/grok-bot/delete-account),
  [voice chat](https://cursor.com/help/grok-bot/voice-chat),
  [pricing](https://cursor.com/pricing),
  [Cursor is now a part of SpaceX](https://cursor.com/blog/joining-spacex).
- The update feed and download headers (`api2.cursor.sh`,
  `downloads.cursor.com`), read 2026-10-05.
- [Grok Bot on the App Store](https://apps.apple.com/us/app/grok-bot/id6794501026).
- [Updates to Full Disk Access in macOS](https://developer.apple.com/news/?id=p6zjojqw),
  Apple, 2 Oct 2026.

Secondary:

- [Homebrew cask `grok-bot`](https://github.com/Homebrew/homebrew-cask/blob/HEAD/Casks/g/grok-bot.rb):
  the bundle id, data folders, minimum macOS and hashes.
- [Release tracker](https://techdevnotes.com/releases/grok-bot-desktop-extra):
  pre-release versions.
- [A Cursor forum thread](https://forum.cursor.com/t/grok-bot-mac-chat-works-local-computer-reported-offline/168973):
  "the Grok Bot desktop app must be open and online to run commands on it".
- SuperGrok prices: press and price trackers only.
