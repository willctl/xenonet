// Writes XenoNet's schemas (core/schema) out as JSON Schema, one file per
// schema at schemas/<path>/v<major>.json, the path of its $id. Files for
// schemas no longer defined are removed. See docs/design/schemas.md.
//
//   npm run schemas              write them
//   npm run schemas -- --check   only say whether they're current (exit 1 if not)
import path from "node:path";
import { schemaFilesState, writeSchemaFiles, SCHEMAS_DIR } from "../core/schema/files.js";

const rel = (f) => path.relative(process.cwd(), path.join(SCHEMAS_DIR, f));

if (process.argv.includes("--check")) {
  const { current, missing, changed, extra } = schemaFilesState();
  for (const [what, files] of [["missing", missing], ["out of date", changed], ["no longer defined", extra]]) for (const f of files) console.log(`${what}: ${rel(f)}`);
  console.log(current ? "the schema files are current" : "run npm run schemas to bring them up to date");
  process.exit(current ? 0 : 1);
}

const { written, removed } = writeSchemaFiles();
for (const f of written) console.log(`wrote ${rel(f)}`);
for (const f of removed) console.log(`removed ${rel(f)}`);
if (!written.length && !removed.length) console.log("the schema files were already current");
