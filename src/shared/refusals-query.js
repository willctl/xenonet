// Query the whole collection before paging. The original API's default is
// still 500 records; the browser requests smaller pages.
export const REFUSAL_SORTS = ["newest", "oldest", "provider", "model", "category", "kind"];
const text = (v) => String(v ?? "");
export const publicRecord = ({ source, key, ...r }) => ({ ...r, source: source ? { file: source.file } : null });
export const modelOf = (r) => r.signal?.model || r.session?.model || "";
export const categoryOf = (r) => r.signal?.category || "";

export function queryRefusals(records, q = {}) {
  const all = records.filter((r) => r && typeof r.id === "string" && Number.isFinite(r.at));
  const facet = (fn) => [...new Set(all.map(fn).filter(Boolean))].sort((a, b) => text(a).localeCompare(text(b)));
  const facets = { provider: facet((r) => r.provider), model: facet(modelOf), category: facet(categoryOf), kind: facet((r) => r.kind) };
  const words = text(q.q).trim().toLowerCase().split(/\s+/).filter(Boolean);
  const since = Number(q.since) || 0;
  const until = Number(q.until) || Infinity;
  const found = all.filter((r) => {
    if (r.at < since || r.at > until) return false;
    if (q.provider && r.provider !== q.provider && r.app !== q.provider) return false;
    if (q.kind && r.kind !== q.kind) return false;
    if (q.model && modelOf(r) !== q.model) return false;
    if (q.category && categoryOf(r) !== q.category) return false;
    if (q.context === "with" && !r.context || q.context === "without" && r.context) return false;
    const haystack = [r.id, r.provider, r.app, r.kind, modelOf(r), categoryOf(r), r.signal?.text, r.signal?.explanation, r.signal?.requestId, r.signal?.fallbackModel, r.session?.id, r.session?.cwd, r.source?.file].map(text).join(" ").toLowerCase();
    return words.every((w) => haystack.includes(w));
  });
  const sort = REFUSAL_SORTS.includes(q.sort) ? q.sort : "newest";
  found.sort((a, b) => {
    const field = sort === "model" ? modelOf : sort === "category" ? categoryOf : (r) => r[sort];
    const byField = ["newest", "oldest"].includes(sort) ? 0 : text(field(a)).localeCompare(text(field(b)));
    return byField || (sort === "oldest" ? a.at - b.at : b.at - a.at) || a.id.localeCompare(b.id);
  });
  const limit = Number.isFinite(Number(q.limit)) ? Math.min(500, Math.max(1, Math.floor(Number(q.limit) || 500))) : 500;
  const offset = Number.isFinite(Number(q.offset)) ? Math.max(0, Math.floor(Number(q.offset) || 0)) : 0;
  return { total: all.length, matched: found.length, offset, limit, facets, records: found.slice(offset, offset + limit).map(publicRecord) };
}
