// Versioned documents XenoNet keeps, such as state.json: a list of steps, each
// a pure function from one version's shape to the next one's, run in order
// from the document's version up to the one wanted. A document is never
// taken back to an older version.
//
//   { from: 1, to: 2, up: (doc, context) => newDoc }
export function migrate(doc, steps, { from, to, context = {} }) {
  if (!Number.isInteger(from) || !Number.isInteger(to)) throw new Error(`versions are whole numbers, not ${from} and ${to}`);
  if (from > to) throw new Error(`version ${from} can't go back to ${to}`);
  // A copy, so a step can't touch the caller's document.
  let out = structuredClone(doc);
  const applied = [];
  for (let v = from; v < to; ) {
    const step = steps.find((s) => s.from === v);
    if (!step || !(step.to > step.from) || step.to > to) throw new Error(`there's no way from version ${v} to ${to}`);
    out = step.up(out, context);
    applied.push(`${step.from}→${step.to}`);
    v = step.to;
  }
  return { doc: out, applied };
}
