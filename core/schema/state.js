// state/*: state.json, XenoNet's own small store (core/store.js). v1 is the
// file as it was before it had a version; v2 adds the version, the
// workspace's root and Codex's picks for each root (contracts, section 5).
// core/statefile.js migrates one to the other.
//
// Loose like the store: a part it doesn't know is kept and written back.
import { define } from "./registry.js";
import { z, obj, ms, nstr } from "./common.js";
import { IdentityDetail } from "./identity.js";

const Layout = z.record(z.string(), obj({ x: z.number(), y: z.number(), z: z.number() })).describe("Where you dragged nodes, by node id");
const Acks = z.record(z.string(), ms).describe('When you dismissed a session\'s "needs you", by session id');
const Marks = z.record(z.string(), z.unknown()).describe("Once-a-day markers, by name");

const Rules = z
  .record(z.string(), obj({ path: z.string(), as: z.enum(["project", "group", "ignore"]), at: ms }))
  .describe("Your rules for folders in the projects folder, by path key");

// What Codex decided the last time it was asked which folders are projects
// (core/classify.js), and what it was asked.
const Picks = obj({
  ranAt: ms,
  ms: z.number().optional(),
  fingerprint: z.string().optional().describe("Of the folders it was sent, to tell when they've changed"),
  sent: z.int().min(0).optional(),
  by: z.string().optional().describe("Who made them: codex, or a decision model by its id (core/decide.js)"),
  picks: z.array(obj({ path: z.string().describe("Inside the projects folder, with forward slashes"), as: z.enum(["project", "group"]), reason: z.string().optional() })),
});

const Identities = obj({
  names: z.record(z.string(), z.string()).optional().describe("Names you gave identities, by id"),
  hidden: z.record(z.string(), z.literal(true)).optional(),
  pins: z.record(z.string(), obj({ identity: z.string(), at: ms })).optional().describe("Sessions and projects you tied to an identity, by node id"),
  known: z
    .record(z.string(), obj({ kind: z.string(), handle: nstr, detail: IdentityDetail, firstSeen: ms, lastSeen: ms }))
    .optional()
    .describe("Every identity XenoNet has seen, by id"),
  signins: z
    .record(z.string(), z.array(obj({ id: z.string(), from: ms, to: ms })))
    .optional()
    .describe("Who each app was signed in as, over time, by app"),
});

export const StateV1 = define(
  "state",
  1,
  obj({
    version: z.literal(1).optional().describe("Not written: a file without a version is v1"),
    layout: Layout.optional(),
    acks: Acks.optional(),
    marks: Marks.optional(),
    projects: obj({ rules: Rules.optional(), codex: Picks.nullable().optional() }).optional(),
    identities: Identities.optional(),
    theme: z.string().optional().describe("The theme in use, by id"),
  }),
  {
    title: "XenoNet's state, v1",
    description: "state.json before it had a version. Every part is optional: XenoNet filled in what was missing when it read the file.",
  },
);

export const StateV2 = define(
  "state",
  2,
  obj({
    version: z.literal(2),
    workspace: obj({ root: nstr.describe("The projects folder; null for XENONET_PROJECTS_DIR, or else ~/Projects") }),
    layout: Layout,
    acks: Acks,
    marks: Marks,
    projects: obj({
      rules: Rules,
      codexByRoot: z.record(z.string(), Picks).describe("Codex's picks for each projects folder it was asked about, by the folder's path key"),
    }),
    identities: Identities,
    theme: z.string().optional().describe("The theme in use, by id"),
    environment: z.string().optional().describe("The environment around the map, independent of the theme"),
  }),
  {
    title: "XenoNet's state",
    description: "state.json: where you dragged nodes, the alerts you dismissed, the projects folder and how to read it (your rules, and Codex's picks for each root), identities (your names and pins, and the sign-ins XenoNet has seen), and the theme.",
  },
);
