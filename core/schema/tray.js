// tray/*: the tray app's API (docs/design/always-on.md, "The tray app";
// contracts section 14). GET /api/tray/v1/status answers a tray/status/v1.
// GET /api/tray/v1/events sends it again, as `status`, whenever it changes,
// and a tray/event/v1, as `alert`, when something starts or stops waiting
// on you. Both answer only XenoNet's tray app, with the key the component
// socket gave it, and only from outside every agent.
import { define } from "./registry.js";
import { z, obj, ms, count } from "./common.js";
import { HOST_ID } from "./ids.js";
import { KINDS } from "../needs-you.js";

// The most items a status lists; `needsYou.count` counts them all.
export const TRAY_ITEMS = 20;

const HostId = z.string().regex(HOST_ID);
// As network/state/v1 has the gateway's link.
const Link = z.enum(["off", "connecting", "live", "offline", "refused", "removed"]);

const item = () =>
  obj({
    key: z.string().min(1).max(300).describe("The item's key in the Needs you queue (core/needs-you.js): session:<id>, actor:<id>, requests…"),
    kind: z.enum(KINDS),
    id: z.string().max(300).nullable().describe("The session or actor it's about, which open-console selects; null for programs asking to be actors"),
    title: z.string().max(200),
    words: z.string().max(60).describe("What it waits for, after its title, as the console says it: asks, needs permission, asks to join XenoNet…"),
    app: z.string().max(32).nullable().describe("The session's app (claude, codex, grok, opencode), or null"),
    host: HostId.optional().describe("The machine it's on, when it isn't this one"),
    text: z.string().max(500).nullable().describe("The question or what it asks, when XenoNet has it"),
    since: ms.nullable(),
    stale: z.boolean().optional().describe("A wait on another machine that isn't live, which may be over"),
    count: count.optional().describe("For requests: how many programs ask"),
  });

export const TrayStatus = define(
  "tray/status",
  1,
  obj({
    schema: z.literal("tray/status/v1"),
    running: z.literal(true).describe("The service answered. A tray that gets no answer shows XenoNet as stopped"),
    version: z.string().max(40).describe("The service's version"),
    instance: z.string().max(40).nullable().describe("installed, demo, dev, or the instance XenoNet Dev names"),
    url: z.string().max(100).describe("The console in a browser: http://127.0.0.1:<port>/"),
    desktop: z.boolean().describe("Whether XenoNet Desktop is connected to the service now"),
    working: count.describe("Sessions that aren't idle"),
    needsYou: obj({
      count: count.describe("How many wait on you, as the console's strip counts them"),
      items: z.array(item()).max(TRAY_ITEMS).describe("The Needs you queue in its order, the longest wait first"),
    }),
    gateway: obj({
      on: z.boolean(),
      state: z.enum(["off", "starting", "online", "problem"]),
      url: z.string().max(300).nullable().describe("Its public address, when it has one"),
      problem: z.string().max(500).nullable(),
    }).describe("The actor gateway, which actors reach this machine through"),
    network: obj({
      name: z.string().max(64).nullable().describe("The network's name; null when this machine isn't in one"),
      host: HostId.nullable().describe("This machine's id in it"),
      link: Link.describe("This machine's link to the network's gateway"),
      hosts: count.describe("The other machines in it"),
      online: count.describe("How many of them are online"),
    }),
    at: ms,
  }),
  {
    title: "The tray's status",
    description: "What GET /api/tray/v1/status answers XenoNet's tray app: that the service runs, what waits on you in the console's words, the actor gateway's state and this machine's link to its network.",
  },
);

export const TrayEvent = define(
  "tray/event",
  1,
  obj({
    schema: z.literal("tray/event/v1"),
    kind: z.enum(["needs-you", "resolved"]).describe("needs-you: it started waiting on you; resolved: it stopped"),
    item: item(),
    at: ms,
  }),
  {
    title: "A tray alert",
    description: "An alert on GET /api/tray/v1/events: an item of the Needs you queue that started or stopped waiting on you, for the tray's notifications.",
  },
);
