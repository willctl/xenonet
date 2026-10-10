// The schema files in schemas/ read back with zod, as anyone outside XenoNet
// would read them: a file's $refs to the others become local ones, to
// copies of those files. Tests check what XenoNet produces against these as
// well as against the definitions.
import fs from "node:fs";
import path from "node:path";
import * as z from "zod/v4";
import { SCHEMA_BASE, parseSchemaId } from "../../core/schema/ids.js";
import { SCHEMAS_DIR } from "../../core/schema/files.js";

const cache = new Map();
const read = (id) => JSON.parse(fs.readFileSync(path.join(SCHEMAS_DIR, `${id}.json`), "utf8"));

export function fromFile(id) {
  if (cache.has(id)) return cache.get(id);
  const defs = {};
  const inline = (node) => {
    if (Array.isArray(node)) return node.map(inline);
    if (!node || typeof node !== "object") return node;
    const out = {};
    for (const [k, v] of Object.entries(node)) {
      if (k !== "$ref" || typeof v !== "string" || !v.startsWith(SCHEMA_BASE)) {
        out[k] = inline(v);
        continue;
      }
      const dep = parseSchemaId(v).id;
      if (!(dep in defs)) {
        defs[dep] = true;
        const { $schema, $id, $comment, ...rest } = read(dep);
        defs[dep] = inline(rest);
      }
      out.$ref = `#/$defs/${dep.replaceAll("~", "~0").replaceAll("/", "~1")}`;
    }
    return out;
  };
  const { $id, $comment, ...root } = read(id);
  const schema = z.fromJSONSchema({ ...inline(root), $defs: defs });
  cache.set(id, schema);
  return schema;
}
