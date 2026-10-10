# Muse

Actor `actor:muse`, product id `muse`. **Status: observed, presence only,
since 2026-10-08.** Researched on 2026-10-05 for Wave 1
(`w1/actors-research`), read-only. The owner installed Muse 7.0 on the
development Mac on 2026-10-08 and asked for it as an actor; section 8
records what the install showed and what XenoNet does with it.

Claims are marked **official** (Meta's newsroom, help center, terms and
security posts, the App Store), **secondary** (press, the Homebrew cask),
**inferred** (our reading) or **to confirm** (after an install).

## In short

- Muse is Meta's personal agent, launched in the US on 8 Sep 2026. It runs
  in a cloud VM of its own for each person (Muse Secure VM) and keeps
  working with the app closed.
- A Mac app followed in mid-September, and computer use on the Mac came at
  Connect on 23 Sep. On the Mac, Muse reads local data through Full Disk
  Access, acts in apps through Automation (and, for computer use,
  Accessibility and Screen Recording), and takes what it reads into its
  cloud VM.
- XenoNet can see only that the app is running. Muse's work on the Mac
  (reading files, Messages and Mail; driving apps) leaves nothing XenoNet can
  attribute without reading protected system data, which it won't.
- **Recommendation (2026-10-05):** don't install it on the development Mac
  for now. If the owner wants Muse as an actor, try it in the VM lab's
  macOS guest, without Full Disk Access, and model it as presence only.
- **Built (2026-10-08):** the owner installed it on the development Mac,
  and XenoNet observes it, presence only (`core/actors/muse.js`). It can't
  be paired as a remote actor: Muse's custom connectors don't reach MCP
  servers (section 8). It reports itself and messages other actors
  through XenoNet's actor gateway, an MCP server it reaches from Meta's cloud
  with a custom connector (section 8, "Reporting itself").

## 1. What it is

| | |
|---|---|
| Maker | Meta Platforms, Inc. ([announcement](https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/), [terms](https://muse.ai/terms)) (official) |
| What | "A secure, private personal AI agent that proactively helps with people's goals and suggests ideas". It handles email and messages, browses, fills forms, books and buys (checkout through Link by Stripe), keeps memory and scheduled tasks, and makes documents and trackers ("Artifacts"). It has its own computer in the cloud with a file system and a terminal ([design notes](https://introducing.muse.ai/)) (official) |
| Where it runs | **Muse Secure VM**: an isolated Linux VM per person. The agent runs in a restricted cell (`systemd-nspawn`), and separate services check safety, hold credentials, and approve every external action and network request. A **Muse Confidential VM**, meant to keep data from Meta too, is planned for later in 2026 ([security post](https://research.meta.ai/blog/security-and-safety-for-ai-agents-our-approach-with-muse)) (official) |
| Status | Launched 8 Sep 2026 on iOS, Android and [muse.ai](https://muse.ai). The Mac app arrived in mid-September (press: 17 to 18 Sep; its Homebrew cask went from 1.0 on 19 Sep to 4.0 on 25 Sep). Computer use on the Mac was announced at Connect on 23 Sep ([Meta newsroom, Spanish edition](https://about.fb.com/ltam/news/2026/09/todo-lo-que-anunciamos-en-meta-connect-2026/)). iOS app 9.1, 1 Oct 2026 ([App Store](https://apps.apple.com/us/app/muse-from-meta/id6760173601)). The pages we could read don't call the Mac app a beta |
| Platforms | iPhone (iOS 18 or later), Android, the web, and the Mac; AI glasses later (official). The Mac app needs Apple silicon and macOS 15 or later according to the press, or macOS 14 according to the Homebrew cask; no Meta page we could read says (secondary) |
| Plans | **Free** with a usage limit; **Power**, $20 a month (500M Muse tokens a week); **Maximum**, $100 a month (3B a week). Bought on muse.ai or in the mobile app, renewing monthly. Subscriptions are "in limited testing" and not in every location ([Muse subscriptions](https://www.meta.com/help/subscriptions/1021145227643680/)) (official) |
| Regions and age | Rolling out in the US. 18 or older ([terms](https://muse.ai/terms)) (official) |
| Not to be confused with | Muse Code, Meta's coding agent CLI ([Meta for Developers](https://developer.meta.com/ai/products/muse-code/)); the Meta AI app for Mac; and other Mac apps called Muse |

## 2. On a Mac

### The app

- **Download:** from [ai.meta.com/muse/download](https://ai.meta.com/muse/download/)
  (official). The file needs a signed link: Homebrew disabled its cask on
  1 Oct 2026 because it "requires a signed download URL", and an anonymous
  request to the download address gets 403 (secondary; checked
  2026-10-05). The size isn't published; the iOS app is 137 MB.
- **App and bundle id:** `/Applications/Muse.app`, bundle id
  `com.meta.endo` (the Homebrew cask; secondary). The iOS app is
  `com.facebook.hatch`.
- **Built with:** a native app showing web content (its data includes a
  `WebKit` folder; inferred). It updates itself (secondary).
- **Helpers, launch agents, login items:** not documented. The cask only
  quits the app when uninstalling, with no launch agent to remove
  (secondary; to confirm).

### macOS permissions

Meta's help center names three:
[How Muse works with files and apps in your Mac](https://www.meta.com/help/artificial-intelligence/1126304576638594/)
(official). Computer use adds two more (secondary: a
[setup guide](https://app.therundown.ai/guides/how-and-why-to-set-up-meta-muse-desktop);
ChatGPT's computer use needs the same pair).

| Permission | What Muse does with it | Why it matters for privacy | Advice |
|---|---|---|---|
| Full Disk Access | "Find, read or update files" anywhere on the Mac, and read local app data: iMessage, Notes, Reminders, Mail, Calendar | It covers every file, Mail, Messages and browser history. What Muse reads goes into its cloud VM: Meta says reading Messages needs Full Disk Access *and* the Messages connector, and a journalist's Messages history was found synced (secondary). Apple said on 2 Oct 2026 it will make this grant harder because of agents ([Apple](https://developer.apple.com/news/?id=p6zjojqw)) | Refuse |
| Automation | "Perform actions on apps that store information locally, like send messages or delete notes" | Muse can act as you in Messages, Mail and Notes | Refuse |
| Notifications | Tells you when a task finishes | What shows on screen | Allow if wanted |
| Accessibility and Screen Recording | Computer use: seeing the screen and clicking and typing in any app. Meta notes interactions can include "screenshots of your screen" | Whatever is on screen goes to the service, and Muse can operate any app, including XenoNet's window | Refuse on the development Mac |

Inside Muse, each local app is set to **Off**, **Read only** or **Read and
interact** (official). Without Full Disk Access these stay greyed out
(secondary). Deleted files go to the Trash (official). Grants can be removed
in Muse's settings or in System Settings → Privacy & Security (official).

### Security history

Both secondary, from the press:

- **21 Sep 2026:** a security researcher showed that any local process
  could change an undocumented Muse setting and send dictation audio and a
  sign-in token to another server. Meta removed the setting.
- **30 Sep 2026:** after a journalist reported Muse reading their Messages,
  Meta said the Messages integration is opt-in and needs both Full Disk
  Access and the Messages connector.

For XenoNet, this means: never write Muse's preferences, and treat its local
files as secret.

## 3. State, logs and conversations

**In the cloud** (official: the
[security post](https://research.meta.ai/blog/security-and-safety-for-ai-agents-our-approach-with-muse),
[data help](https://www.meta.com/help/artificial-intelligence/2225571704857152/),
[privacy help](https://www.meta.com/help/artificial-intelligence/1047255454427887/)):

- The VM holds the person's files, "everything Muse generates or uses on
  your behalf", and the OAuth tokens of connected services. The agent sees
  only stand-in tokens, swapped for the real ones at the network edge.
- Muse keeps chats, goals, reminders, connector data, and its memory and
  personality files (`Memory.md`, `Soul.md`, `Identity.md`).
- **Settings → Data controls** has **Download your Muse data** and **Reset
  Muse**. Deleting the Meta account deletes Muse's data.
- Training on interactions is on by default; you can opt out in Data
  controls. Muse "doesn't share your conversations or the data in your
  virtual machine with Meta ad systems".

**On the Mac**, nothing is documented by Meta. The Homebrew cask's cleanup
list (secondary):

- `~/Library/Application Support/com.meta.endo`
- `~/Library/Caches/com.meta.endo`
- `~/Library/HTTPStorages/com.meta.endo`
- `~/Library/Preferences/com.meta.endo.plist`
- `~/Library/WebKit/com.meta.endo`

No log folder is known. Treat all of it as secret: the sign-in session
lives there.

## 4. What XenoNet can observe

### Presence

Only that `Muse.app` (bundle `com.meta.endo`) is running. The process name
and signing team id are to confirm. Muse works in its VM whether or not the
app runs, so with the app closed, presence is `unknown`, not `offline`. No
local API or socket is documented.

### The work it does here

| What Muse does on the Mac | Visible to XenoNet? |
|---|---|
| Reading files and app data (Messages, Mail, Notes) through Full Disk Access | No. Seeing file access needs root-level tracing (`fs_usage`) or Full Disk Access for XenoNet itself, and either would be worse than the problem |
| Driving apps through Automation | No. Apple Events aren't logged anywhere XenoNet can read without a privacy cost, and the list of what each app may automate sits in a protected database |
| Computer use (Accessibility, Screen Recording) | No. Clicks and keystrokes it makes can't be traced back to Muse |
| Shell commands | Not documented: Muse's terminal is in its VM. If it ever runs commands on the Mac, process ancestry works as it does for Grok Bot |
| Connectors (GitHub was added at Connect) | Not on the Mac: they act in the services, not here |

How to model it:

1. **Presence only** (recommended): `via: ["observed"]`, no targets, no
   events except presence. Honest about what XenoNet knows.
2. **Through the services Muse acts in**, such as GitHub. That's outside
   XenoNet's scope, and nothing documented marks Muse's actions there.
3. **Nothing**, if Muse isn't installed on a Mac that XenoNet maps.

### Proposed snapshot

| Field | Value |
|---|---|
| `id`, `kind`, `name` | `actor:muse`, `agent`, "Muse" |
| `product` | `{ "id": "muse", "vendor": "Meta", "version": <CFBundleShortVersionString> }` |
| `via` | `["observed"]` |
| `presence` | `idle` while the app runs, `unknown` otherwise; never `working`, since XenoNet can't see work |
| `status` | null |
| `capabilities` | `browse`, `message-people`, `schedule` (from Meta's docs) |
| `targets`, `recent` | Empty |
| `identity` | null (section 6) |

## 5. How it acts, and for whom

All official ([guidance and approval](https://www.meta.com/help/artificial-intelligence/1385290430137537/),
the [security post](https://research.meta.ai/blog/security-and-safety-for-ai-agents-our-approach-with-muse),
[Connect 2026](https://about.fb.com/news/2026/09/the-biggest-news-from-connect-2026/)).

- **For whom:** the person, through their own connected accounts. Muse
  sends messages and email, buys and negotiates, and fills forms.
  - At Connect, Meta said "Muse will have its own email address that it can
    use to get things done".
  - Payments use single-use card numbers, and every purchase needs a human
    approval.
- **Approval setting:** **Ask for some actions** (writes and important
  reads) or **Always ask**.
- **Approval choices:** **Allow once**, **Allow for this task**, **Allow
  for this site**, **Always allow** (for that connector) and **Deny**.
- **Enforcement:** a permission service outside the model ("Sentinel")
  decides which actions go ahead, ask, or are refused.

## 6. Sign-in and identity

- **Account:** a Meta account. You can create one with an existing
  Facebook or Instagram account, or with an email address or phone number
  ([get started](https://www.meta.com/help/artificial-intelligence/1331373868832401/))
  (official).
- **Keeping it separate:** you can keep Muse apart by "signing up with an
  email address that is not associated with another Meta account". Adding
  Muse to Accounts Center is optional; if you do, Meta may combine
  information across profiles ([terms](https://muse.ai/terms)) (official).
- **What the app exposes:**
  - the Meta account's profile and its name (inferred), and through
    Accounts Center, other Meta profiles;
  - Muse's own identity and personality files;
  - the email address Muse sends from. What name it shows recipients isn't
    documented.

  The App Store's privacy label lists identifiers, contacts and location
  among data linked to you.
- **For XenoNet:** `identity` stays null. XenoNet reads nothing of Muse's
  account and never shows its name.

## 7. Install plan (short)

**Decision at merge point A.** Recommended: **not now** on the development
Mac. Muse would add the most invasive permissions of the three actors, and
XenoNet would see the least.

If the owner says yes:

1. **Where:** the VM lab's macOS guest first, not the development Mac.
2. **Requirements:** Apple silicon, macOS 15 or later (press; Homebrew says
   14), the US, 18 or older, and a Meta account (see section 6).
3. **Cost:** the free tier is enough to test.
4. **Download:** the owner signs in at [muse.ai](https://muse.ai) and
   downloads from [ai.meta.com/muse/download](https://ai.meta.com/muse/download/).
   The size is unknown until then.
5. **Install:** drag `Muse.app` to `/Applications`, open it and sign in
   (owner only).
6. **Refuse** Full Disk Access, Automation, Accessibility and Screen
   Recording; allow notifications if wanted (owner).
7. **In Muse:** set every local app to **Off**, and **Always ask** for
   approvals. Under Data controls, turn off AI training if wanted (owner).
8. **Record, read-only:** `ps` with the app open, the bundle id and team id
   (`codesign -dv`), and the names of its folders (agent).
9. **Uninstall:** quit Muse, then move `Muse.app` and the five folders of
   section 3 to the Trash. Remove its grants in Privacy & Security. For the
   cloud side, use **Reset Muse**, or delete the Meta account. Cancel any
   subscription first.

## 8. After the install (2026-10-08)

Recorded read-only on the development Mac, with the app open and signed
in: names and shapes only. Nothing under Muse's folders was opened; of the
app, only its `Info.plist`, its bundled web app's strings, and its public
update feed were read.

| | |
|---|---|
| App | `/Applications/Muse.app`, version 7.0 (build 1086317620), bundle id `com.meta.endo`, team id `V9WTTPBFK9` (`codesign -dv`). A universal binary, Apple silicon and Intel, with `LSMinimumSystemVersion` 14.0. This settles the open question of section 2: macOS 14, as Homebrew said |
| Built with | A native app showing a bundled web app (`Contents/Resources/hatch/index.html`, 45 MB) in WebKit. Meta's own names for it are Endo (the Mac app) and Hatch (the web app and the iOS app) |
| Processes | `Contents/MacOS/Muse`, and its crash reporter `Contents/MacOS/EndoCrashpadHandler`, both under launchd. Nothing ran under the app while it sat open |
| Helpers, launch agents, login items | None: no XPC services or app extensions in the bundle, and nothing of Meta's in `~/Library/LaunchAgents`. Sparkle's `Updater.app` is inside the bundle |
| Updates | Sparkle, from a feed anyone can read: `https://www.facebook.com/endo/release/appcast.xml?channel=production`, the app's `SUFeedURL`. On 2026-10-08 it listed 7.0, published on 7 Oct. The provider drift check reads it (`meta/muse-desktop`) |
| Network | One TLS connection to Meta. No local port or socket that another program could talk to |
| Folders | The five of section 3, and `~/Library/Application Support/hatch/` (`AvatarCache`, `node-id.txt`). `Application Support/com.meta.endo` holds analytics and a storage folder; `Caches/com.meta.endo` a URL cache, WebKit's caches, `AuthRollout`, `MobileConfig` and `Crashpad`; `HTTPStorages` its cookies; `WebKit/com.meta.endo` the web app's IndexedDB and local storage. No log folder, and no conversation files: chats live in the cloud |

Two cautions:

- `~/Library/Application Support/hatch` is also the folder of Hatch, the
  Python project tool. On this Mac it held that tool's own `config.toml`
  and environments from 2024 beside Muse's two entries. To uninstall Muse,
  remove only `AvatarCache` and `node-id.txt` there, not the folder.
- The web app's storage and cookies hold the sign-in. XenoNet reads none of
  them, nor anything else in those folders.

Which permissions the owner granted wasn't checked: macOS keeps that in a
database XenoNet can only read with Full Disk Access.

### Custom connectors

Muse 7.0 can add custom connectors (inferred, from the strings and request
shapes in its bundled web app). A connector is either:

- an **API key** for a list of hosts, which Meta's token proxy adds to
  requests to those hosts, so the agent never sees it; or
- **OAuth**: authorization, token and revocation URLs, a client id and
  secret, or dynamic client registration.

Nothing in the app mentions MCP. So Muse can't call XenoNet's read-only
tools (`xenonet mcp assistant`) the way a dot can through OpenAI's tunnel,
and it isn't paired as a remote actor (`remote: false` in
`core/actors/products.js`). Pairing it another way would mean serving an
HTTP API that Meta's cloud can reach, which XenoNet doesn't do.

### What XenoNet does

`core/actors/muse.js`, as section 4 proposed:

- `actor:muse`, `via: ["observed"]`, `heartbeatMs` null, identity null.
  Capabilities `browse`, `message-people`, `schedule`, and `use-computer`
  for computer use on the Mac.
- `idle` while the app's main program runs ("Its app is open on this Mac;
  it works in Meta's cloud"), `unknown` otherwise; never `working` and
  never `offline`. It stays listed while the app is installed, as Grok Bot
  does. The crash reporter and the updater don't count as the app open.
- No events or targets beyond presence and status, even when something
  runs under the app.
- It reads the process table, shared with the other observers, and two
  keys of the `Info.plist`: `CFBundleShortVersionString` and
  `CFBundleIdentifier`, through `plutil`, since the file is binary. With
  its observer off, it reads nothing.
- A `Muse.app` is Meta's only when its bundle id is `com.meta.endo`, in
  the observer and in `core/procs.js`, as a Hermes.app is checked. Another
  maker's Muse.app is no product at all.
- Its own processes are refused for actors' tokens, as Grok Bot's are
  (interface.md, section 5, rule 3).

### Reporting itself (2026-10-08)

The owner asked for Muse and Grok Bot to talk through XenoNet.

- **No local route.** Muse's code has a "Run commands" permission
  (`system.run`), but the owner's account isn't offered it: with File
  system access on and Computer use set to ask, Muse said its pairing with
  the Mac has no program runner, and it ran nothing here. Typing a command
  into Terminal through computer use would look like the owner to XenoNet,
  so that route is refused by design. XenoNet's process check would refuse a
  muse token from anywhere but under Muse's app as Meta signed it (team
  `V9WTTPBFK9`, checked with `codesign`), should Meta enable it later.
- **The gateway.** Muse's custom connectors reach HTTPS hosts with an API
  key that Meta's token proxy adds (as a bearer header, among others). So
  Muse reaches XenoNet's actor gateway (`server/actor-gateway.js`): an MCP
  server, over HTTP, that the owner turns on and puts on the internet with
  a tunnel they pick (`gateway.md`). Muse's agent posts JSON-RPC to it from its cloud
  computer (`tools/call` with `send`, `inbox`, `peers`, `whoami`,
  `heartbeat`). Making Muse's gateway token in XenoNet's window registers
  `actor:muse`, joining the actor XenoNet observes. Its messages reach agents
  marked as coming by a gateway token from outside this machine (weak, the
  token alone). The file mailbox Muse's file access would allow was turned
  down by the owner in favour of this.

### Routes to more, later

1. **Commands under the app.** Grok Bot's process ancestry (`ran`,
   `spawned`) could apply to what Muse runs, once the real-system test
   shows where its programs run.
2. **A remote connection.** Once Muse's custom connectors can reach an MCP
   server, add an adapter for it (`core/actors/adapters.js`) and set
   `remote: true`. It would still need a route from Meta's cloud to this
   Mac, as OpenAI's tunnel is for dots.

## The Phase 0 notes, checked

| Phase 0 said | Now |
|---|---|
| A Mac app, in beta, needing macOS 15 and Apple silicon (unverified) | **The app is verified:** Meta's help center covers it. "Beta" appears on no Meta page we could read. macOS 15 and Apple silicon come from the press only; the Homebrew cask says macOS 14 |
| Computer use ("drive any app on your Mac") added at Connect on 23 Sep (unverified) | **Verified** in Meta's newsroom |
| It asks for Full Disk Access, Automation and Notifications (unverified) | **Verified** in Meta's help center. Computer use adds Accessibility and Screen Recording (secondary) |
| Each local app set to Off, Read only, or Read and interact (unverified) | **Verified** in Meta's help center |
| Free, with Power at $20 and Maximum at $100 a month (unverified) | **Verified** in Meta's help center, with 500M and 3B Muse tokens a week |
| It needs a Meta account | **Verified.** It can be a new one made with an email address, kept apart from other Meta accounts |
| A download page whose title mentions Mac | It renders in the browser, and the file needs a signed link. Homebrew disabled its cask on 1 Oct 2026 for that reason |

## Open questions

- Muse at all, and where? Answered on 2026-10-08: the owner installed it on
  the development Mac and asked for it as an actor.
- Is presence alone worth an actor? Built, presence only.
- After an install: answered in section 8, except whether Muse ever runs
  commands on the Mac.
- When Muse's custom connectors reach MCP servers, should it be paired as a
  remote actor?

## Sources

Official:

- [Introducing Muse](https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/),
  Meta, 8 Sep 2026.
- [The biggest news from Connect 2026](https://about.fb.com/news/2026/09/the-biggest-news-from-connect-2026/),
  and its [Spanish edition](https://about.fb.com/ltam/news/2026/09/todo-lo-que-anunciamos-en-meta-connect-2026/),
  which says computer use is available in Muse for Mac.
- [Security and safety for AI agents: our approach with Muse](https://research.meta.ai/blog/security-and-safety-for-ai-agents-our-approach-with-muse).
- Meta help center:
  [Muse help center](https://www.meta.com/help/artificial-intelligence/1303670544995562/),
  [files and apps in your Mac](https://www.meta.com/help/artificial-intelligence/1126304576638594/),
  [guidance and approval](https://www.meta.com/help/artificial-intelligence/1385290430137537/),
  [manage your data](https://www.meta.com/help/artificial-intelligence/2225571704857152/),
  [privacy, safety and security](https://www.meta.com/help/artificial-intelligence/1047255454427887/),
  [get started](https://www.meta.com/help/artificial-intelligence/1331373868832401/),
  [subscriptions](https://www.meta.com/help/subscriptions/1021145227643680/).
- [Muse terms](https://muse.ai/terms), [How we designed Muse](https://introducing.muse.ai/),
  [download page](https://ai.meta.com/muse/download/).
- [Muse from Meta on the App Store](https://apps.apple.com/us/app/muse-from-meta/id6760173601).
- [Updates to Full Disk Access in macOS](https://developer.apple.com/news/?id=p6zjojqw),
  Apple, 2 Oct 2026.

Secondary:

- [Homebrew cask `muse`](https://github.com/Homebrew/homebrew-cask/blob/HEAD/Casks/m/muse.rb)
  and its history: bundle id, folders, versions, disabled on 1 Oct 2026.
- [TechCrunch, 18 Sep 2026](https://techcrunch.com/2026/09/18/metas-muse-hits-mac-letting-the-ai-take-actions-on-your-computer/):
  the Mac launch.
- [The Rundown's setup guide](https://app.therundown.ai/guides/how-and-why-to-set-up-meta-muse-desktop):
  Accessibility and Screen Recording for computer use.
- [Decrypt](https://decrypt.co/379868/meta-muse-ai-agent-read-messages-without-permission):
  the Messages dispute and Meta's response.
- [Gizmodo](https://gizmodo.com/meta-just-patched-a-major-zero-day-vulnerability-in-its-muse-ai-assistant-2000815429):
  the dictation setting and its fix.
