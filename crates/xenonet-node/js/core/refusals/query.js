// core/refusals/query.js in Rust (crates/xenonet-sources/src/refusals/query.rs):
// the refusals browser's query over every record.
import { native } from "../../index.js";

export const REFUSAL_SORTS = native.refusalSorts();
export const publicRecord = (r) => native.refusalPublicRecord(r);
export const modelOf = (r) => native.refusalModelOf(r);
export const categoryOf = (r) => native.refusalCategoryOf(r);

export function queryRefusals(records, q = {}) {
  if (!Array.isArray(records)) throw new TypeError("records.filter is not a function");
  if (q === null) throw new TypeError("Cannot read properties of null (reading 'q')");
  return native.queryRefusals(records, q);
}
