// The one fix the drift check makes by itself: a schema's range of vendor
// versions, widened to take a version the check read without finding a
// change to what XenoNet reads. The range only says which versions a schema
// was made for, so widening it keeps every version it had (older hosts
// still read with the same schema) and adds the new one. Anything more
// (a field to read, a kind to pass over, a new major) is a person's to
// decide, from the report.
import { parseVersion, compareVersions, satisfies, validRange } from "../../core/vendor.js";

// "<0.4.0" → [0, 4, 0]: a comparator's version, or null.
const bound = (text) => {
  const v = parseVersion(text);
  return v && v[3] == null ? v.slice(0, 3) : null;
};

// The next bound past `v`, as fine as `upper` is: a major (<2.0.0), a
// minor (<0.4.0) or a patch (<0.3.7).
function nextBound(v, upper) {
  if (upper[1] === 0 && upper[2] === 0 && upper[0] > 0) return [v[0] + 1, 0, 0];
  if (upper[2] === 0) return [v[0], v[1] + 1, 0];
  return [v[0], v[1], v[2] + 1];
}

// `range` widened to take `version`: its upper bound moved past it, or its
// lower bound down to it, keeping the range's form ("<1.0.0",
// ">=0.3.0 <0.4.0", ">=1.0.0 <3.0.0"). The range as it is when it takes the
// version already; null when the range has another form (alternatives,
// x-ranges, a version of the data's own format), which a person widens.
export function widenRange(range, version) {
  const v = parseVersion(version);
  if (!v || !validRange(range)) return null;
  if (satisfies(version, range)) return range;
  if (String(range).includes("||")) return null;
  const parts = String(range).trim().split(/\s+/);
  const lower = parts.find((p) => p.startsWith(">="));
  const upper = parts.find((p) => p.startsWith("<") && !p.startsWith("<="));
  if (parts.length !== [lower, upper].filter(Boolean).length) return null;
  const lo = lower ? bound(lower.slice(2)) : null;
  const hi = upper ? bound(upper.slice(1)) : null;
  if ((lower && !lo) || (upper && !hi)) return null;
  const core = [v[0], v[1], v[2], null];
  let newLo = lo;
  let newHi = hi;
  if (lo && compareVersions(core, [...lo, null]) < 0) newLo = [v[0], v[1], 0];
  if (hi && compareVersions(core, [...hi, null]) >= 0) newHi = nextBound(v, hi);
  const out = [newLo && `>=${newLo.join(".")}`, newHi && `<${newHi.join(".")}`].filter(Boolean).join(" ");
  return satisfies(version, out) ? out : null;
}

// The source of a schema's definition (core/schema/vendor/<vendor>/<product>.js)
// with the `versions:` of the schema for `source` set to `to`: the first
// `versions: "…"` after the source's id, before the next vendor(…) call.
// Null when it isn't there, or doesn't say `from`.
export function setVersionsInSource(text, source, from, to) {
  const at = text.indexOf(`"${source}"`);
  if (at < 0) return null;
  const next = text.indexOf("vendor(", at + source.length + 2);
  const end = next < 0 ? text.length : next;
  const re = /versions:\s*("(?:[^"\\]|\\.)*")/g;
  re.lastIndex = at;
  const m = re.exec(text);
  if (!m || m.index > end || JSON.parse(m[1]) !== from) return null;
  const start = m.index + m[0].length - m[1].length;
  return text.slice(0, start) + JSON.stringify(to) + text.slice(start + m[1].length);
}

// docs/design/schemas.md's table of sources, with the range in the row for
// `source` set to `to`: the row's last cell, `<range>`. The text as it was
// when the row isn't there or doesn't say `from`.
export function setVersionsInDocs(text, source, from, to) {
  const lines = text.split("\n");
  const i = lines.findIndex((l) => l.startsWith(`| \`${source}\` |`));
  if (i < 0) return text;
  const cell = `| \`${from}\` |`;
  if (!lines[i].endsWith(cell)) return text;
  lines[i] = lines[i].slice(0, -cell.length) + `| \`${to}\` |`;
  return lines.join("\n");
}
