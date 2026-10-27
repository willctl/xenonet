// How live delivery reaches each app's sessions: the apps the registry gives
// `delivery`, each with its own ways in here, by its id.
//
//   resume(id, { settings })  the command for one headless turn of a session
//                             whose process has gone (resume.js), as
//                             [command, args]
//   marks                     the start of the variables that mark a process
//                             as one of the app's sessions, which a resumed
//                             turn leaves out; where the app keeps its
//                             sessions (the registry's roots.env) stays
//   channel                   a session started with XenoNet as a channel
//                             takes messages pushed into it (channel.js)
//   server                    the app's own server takes a message for a
//                             session: OpenCode's, which the router reaches
//                             through opencode.js
import { APPS } from "../apps.js";

export const DELIVERY = {
  // claude -p --resume <id> [--settings <listen file>]
  claude: { resume: (id, { settings = null } = {}) => ["claude", ["-p", "--resume", id, ...(settings ? ["--settings", settings] : [])]], marks: "CLAUDE", channel: true },
  // codex exec resume <id> -
  codex: { resume: (id) => ["codex", ["exec", "resume", id, "-"]], marks: "CODEX_" },
  // grok -r <id> --output-format json --prompt-file <file>: Grok Build reads
  // a single turn's prompt from a file, which resume.js writes and adds.
  grok: { resume: (id) => ["grok", ["-r", id, "--output-format", "json", "--prompt-file"]], marks: "GROK_" },
  opencode: { server: true },
};

// The apps live delivery reaches, in the registry's order, each with its
// ways in and where it keeps its sessions. A test can give its own
// registry and ways.
export const deliveryApps = (registry = APPS, ways = DELIVERY) => registry.filter((a) => a.has?.includes("delivery") && Object.hasOwn(ways, a.id)).map((a) => ({ id: a.id, name: a.name, roots: a.roots, ...ways[a.id] }));

const APPS_IN = deliveryApps();

// An app's ways in, by its id: none for anything but an id it has.
export const waysOf = (app) => (typeof app === "string" ? (APPS_IN.find((a) => a.id === app) ?? null) : null);

// Whether a session's id is an app's whose own server takes messages.
export const served = (id) => APPS_IN.some((a) => a.server && String(id).startsWith(`${a.id}:`));
