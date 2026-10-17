// core/schema/registry.js with its bookkeeping in Rust
// (crates/xenonet-schema/src/registry.rs): the ids defined, the lookups by
// either form of id, and the documents npm run schemas writes, in order.
// zod stays here: the schemas themselves, .meta(), its registry, and
// z.toJSONSchema, which writes the JSON Schema that Rust puts in order.
import * as z from "zod/v4";
import { native } from "../../index.js";
import { shortId, schemaUri } from "./ids.js";

const registry = z.registry();
const byId = new Map();
const rust = new native.SchemaRegistry();
// What z.toJSONSchema last wrote, until another schema is defined.
let written = false;

export function define(path, major, schema, meta) {
  const id = shortId(path, major);
  const refused = rust.refusal(id, Boolean(meta?.title), Boolean(meta?.description));
  if (refused) throw new Error(refused);
  const defined = schema.meta(meta);
  registry.add(defined, { id });
  byId.set(id, defined);
  rust.add(id);
  written = false;
  return defined;
}

export function schemaFor(id) {
  const short = rust.find(String(id ?? ""));
  const found = short == null ? undefined : byId.get(short);
  if (!found) throw new Error(`XenoNet has no schema ${id}`);
  return found;
}

export const schemaIds = () => rust.ids();

function write() {
  if (written) return;
  rust.setDocuments(z.toJSONSchema(registry, { uri: schemaUri }).schemas);
  written = true;
}

export function jsonSchemas() {
  write();
  return rust.documents();
}

export function schemaFiles() {
  write();
  return rust.files();
}
