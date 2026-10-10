// doctor/*: the doctor's checks and runs (core/doctor), as GET and POST
// /api/doctor, the `doctor` event and doctor.json have them.
import { define } from "./registry.js";
import { z, obj, ms, count, nstr } from "./common.js";

export const DoctorResult = define(
  "doctor/result",
  1,
  obj({
    id: z.string().describe("<group>.<check>: xenonet.server, claude.auth, agents.checked…"),
    group: z.string(),
    title: z.string(),
    status: z.enum(["ok", "info", "warn", "fail", "skip"]).describe("info is a fact, not a problem; skip means the check couldn't run here"),
    detail: z.string(),
    fix: z.string().optional(),
    ms: z.number(),
  }),
  {
    title: "A check",
    description: "One of the doctor's checks: whether something XenoNet needs is in place, and how to fix it when it isn't.",
  },
);

export const DoctorRun = define(
  "doctor/run",
  1,
  obj({
    schema: z.literal("doctor/run/v1").optional().describe("In doctor.json"),
    at: ms,
    ms: z.number(),
    results: z.array(DoctorResult),
    summary: obj({ ok: count, info: count, warn: count, fail: count, skip: count }),
    by: nstr.optional().describe("Which XenoNet ran it, when the xenonet command asked one; null when it ran in the command itself"),
  }),
  {
    title: "A doctor run",
    description: "Every check the doctor ran, and how many came out each way.",
  },
);
