// XenoNet's schemas, by id. Each area's module (session.js, graph.js, …)
// defines its schemas here, and `npm run schemas` writes them out as JSON
// Schema files in schemas/, one per id. The definitions are the source of
// truth; the files are what anyone outside XenoNet reads (see
// docs/design/schemas.md and docs/design/adr/0001-schemas.md).
import * as z from "zod/v4";
import { shortId, schemaUri, parseSchemaId } from "./ids.js";

// Ids only. Titles, descriptions and x-xenonet go on the schemas themselves
// (.meta()), since the writer reads nested ones from there.
const registry = z.registry();
const byId = new Map();

// Registers `schema` as <path>/v<major>, with a title and a description
// (and anything else for the file, such as x-xenonet), and returns it as
// registered: use what this returns, since .meta() makes a new schema.
export function define(path, major, schema, meta) {
  const id = shortId(path, major);
  if (byId.has(id)) throw new Error(`${id} is defined twice`);
  if (!meta?.title || !meta?.description) throw new Error(`${id} needs a title and a description`);
  const defined = schema.meta(meta);
  registry.add(defined, { id });
  byId.set(id, defined);
  return defined;
}

// The schema with this id, in either form (actor/snapshot/v1, or its $id).
export function schemaFor(id) {
  const found = byId.get(parseSchemaId(id)?.id);
  if (!found) throw new Error(`XenoNet has no schema ${id}`);
  return found;
}

export const schemaIds = () => [...byId.keys()].sort();

const COMMENT = "Written by npm run schemas from core/schema. Change the definition there, not this file.";

// Every schema as JSON Schema (draft 2020-12), by id: each with its $id,
// and a $ref to the other's $id where one schema uses another.
export function jsonSchemas() {
  const { schemas } = z.toJSONSchema(registry, { uri: schemaUri });
  return Object.fromEntries(
    Object.keys(schemas)
      .sort()
      .map((id) => {
        const { $schema, $id, title, description, ...rest } = schemas[id];
        return [id, { $schema, $id, $comment: COMMENT, title, description, ...rest }];
      }),
  );
}

// The files `npm run schemas` writes, by path under schemas/.
export const schemaFiles = () => Object.entries(jsonSchemas()).map(([id, json]) => ({ file: `${id}.json`, text: `${JSON.stringify(json, null, 2)}\n` }));
