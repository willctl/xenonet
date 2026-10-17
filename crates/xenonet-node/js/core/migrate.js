// core/migrate.js in Rust (crates/xenonet-state/src/migrate.rs): a
// document moved up a step at a time, never back. Rust says which steps
// take it from its version to the one wanted, or why none do; the steps
// themselves are the caller's functions, run here on a copy.
import { native } from "../index.js";

export function migrate(doc, steps, { from, to, context = {} }) {
  const plan = native.planMigration(
    steps.map((s) => [Number(s.from), Number(s.to)]),
    from,
    to,
  );
  // A copy, so a step can't touch the caller's document.
  let out = structuredClone(doc);
  const applied = [];
  for (const i of plan) {
    const step = steps[i];
    out = step.up(out, context);
    applied.push(`${step.from}→${step.to}`);
  }
  return { doc: out, applied };
}
