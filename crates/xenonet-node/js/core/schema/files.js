// core/schema/files.js in Rust (crates/xenonet-schema/src/files.rs): the
// files in schemas/ against what the definitions write, and writing them.
// The texts come from schemaFiles(); Rust lists, compares, writes and
// removes.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { native } from "../../index.js";
import { schemaFiles } from "./index.js";

export const SCHEMAS_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..", "..", "schemas");

export const schemaFilesState = (dir = SCHEMAS_DIR) => native.schemaFilesState(String(dir), schemaFiles());

export const writeSchemaFiles = (dir = SCHEMAS_DIR) => native.writeSchemaFiles(String(dir), schemaFiles());
