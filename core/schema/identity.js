// identity/v1: an account work happens under, as the map lists it
// (graph.identities, from linkIdentities in core/identities.js), and the
// links that tie sessions and projects to one.
import { define } from "./registry.js";
import { z, obj, ms, count, nstr } from "./common.js";

export const CONFIDENCE = ["yours", "exact", "likely", "assumed"];

// Who a session ran as.
export const RanAs = obj({
  id: z.string(),
  confidence: z.enum(CONFIDENCE).describe("yours: you pinned it; exact: the app recorded it; likely: signed in at the time; assumed: the only account XenoNet has seen"),
  via: z.string().describe("How XenoNet knows, in words"),
});

// Whose a project is, who pushes to it, and who its commits are by.
export const ProjectIdentity = obj({
  id: z.string(),
  role: z.enum(["owner", "pushes-as", "commits-as"]),
  confidence: z.enum(CONFIDENCE),
  via: z.string(),
});

// What XenoNet knows about the account besides its handle.
export const IdentityDetail = obj({
  plan: nstr.optional(),
  org: nstr.optional().describe("A Claude team's or enterprise's name"),
  host: z.string().optional().describe("A GitHub account's host"),
  name: nstr.optional().describe("The name git commits carry"),
});

export const Identity = define(
  "identity",
  1,
  obj({
    id: z.string().describe("claude:<account>:<organization>, chatgpt:<workspace>, openai:api-key, github:<host>/<login> or git:<hash of the email>. Identities have no host: the ids are already global"),
    kind: z.enum(["claude", "chatgpt", "openai", "github", "git"]),
    handle: nstr.describe("The email or login it shows as"),
    detail: IdentityDetail,
    signedIn: z.boolean(),
    active: z.boolean().describe("gh's active account for its host"),
    sources: z.array(z.string()).describe("Where XenoNet found it, with the home folder as ~"),
    firstSeen: ms,
    seenAt: ms.optional(),
    name: nstr.describe("The name you gave it; identities with the same name show as one"),
    hidden: z.boolean(),
    sessions: count,
    projects: count,
    hosts: z.array(z.string().nullable()).optional().describe("With hosts on: the machines it was seen on, null for this one"),
    signedInOn: z.array(z.string().nullable()).optional().describe("With hosts on: the machines it's signed in on, null for this one"),
  }),
  {
    title: "Identity",
    description: "An account work happens under: a Claude account in an organization, a ChatGPT workspace, an OpenAI API key, a GitHub account, or the email git commits with.",
  },
);
