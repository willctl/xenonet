// drift/*: differences between vendor data and its schema, kept as shapes
// and never values (contracts, section 1). The lenient readers keep them
// in drift.json in the cache folder (drift/log), a drift/record each; xenonet
// drift, npm run contracts and the doctor report them.
import { define } from "./registry.js";
import { z, obj, ms, nstr } from "./common.js";

export const DriftRecord = define(
  "drift/record",
  1,
  obj({
    schema: z.literal("drift/record/v1"),
    source: z.string().describe("<vendor>/<product>/<source>, the vendor schema's path without vendor/: anthropic/claude-code/transcript-line"),
    vendorVersion: nstr.describe("The version of the vendor's app that wrote the data, if it says"),
    kind: z.enum(["new-field", "missing-field", "type-change", "new-variant", "new-version", "unparsable"]),
    path: nstr.describe("Where in the record: message.content[].citations; null where there's no field, as for a line that isn't JSON"),
    variant: z
      .array(z.union([z.string(), z.number(), z.boolean()]))
      .min(1)
      .max(8)
      .optional()
      .describe("The kind of record it's in: the values of the discriminators around it, outermost first, such as [\"assistant\", \"text\"] for a text block in a reply; only values the schema lists. Left out where there's none"),
    shape: nstr.describe("Types only, at most four levels deep: object keys and their value types, array element types. Never a value, except a declared discriminator's (a line's type) and vendor versions"),
    firstSeen: ms,
    lastSeen: ms,
    count: z.int().min(1),
  }),
  {
    title: "A drift record",
    description: "Vendor data that differs from XenoNet's schema for it: a new or missing field, a field of another type, a new variant, a new version, or a line that isn't JSON.",
  },
);

export const DriftLogFile = define(
  "drift/log",
  1,
  obj({
    version: z.literal(1).describe("drift.json's own version; its records each say what they are"),
    lookedAt: ms.nullable().describe("When you last looked at the drift (xenonet drift, or the console; never npm run contracts): what's first seen after it is new, and the doctor says so"),
    records: z.array(DriftRecord).max(500).describe("The drift kept, merged across vendor versions, at most 500: the ones seen least lately go first"),
  }),
  {
    title: "XenoNet's drift log",
    description: "drift.json in the cache folder: the drift the lenient readers have found in the other apps' data, merged across the processes that write it (the server, xenonet drift, npm run contracts), and when you last looked.",
  },
);
