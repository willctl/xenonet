// What vendor schemas are made of (contracts, section 1). A vendor schema
// says what XenoNet reads of another app's data: the fields it reads, with
// their types, and nothing else. XenoNet reads it leniently (see
// core/schema/lenient.js and core/vendor.js): a record that doesn't fit is
// still used for the fields that do, and every difference becomes a drift
// record (core/drift.js).
//
// Schemas are made only from what XenoNet's code reads and from the vendors'
// public formats, never from anyone's data: a version range, a field's type
// and whether it's always there may be checked against real data, but no
// name or value is copied from it.
import { define } from "../registry.js";
import { z } from "../common.js";

export { z };

// A vendor schema, at vendor/<vendor>/<product>/<source>/v<major>, with the
// versions of the vendor's app (or of the data's own format) it was made
// for, as a semver range ("*" where the data carries no version): when a
// vendor changes a format, the schema gets a new major with its own range,
// and the reader picks by the version it finds. The description says where
// that version comes from.
export function vendor(path, major, schema, { title, description, versions }) {
  const own = schema.meta()?.["x-xenonet"] ?? {};
  return define(`vendor/${path}`, major, schema, { title, description, "x-xenonet": { ...own, vendorVersions: versions, lenient: true } });
}

// Objects are open: other keys pass without a word, since vendor records
// are large, XenoNet reads a few of their fields, and the rest changes with
// every release. `closed` marks the small objects whose keys are all the
// format's (token usage, a rate limit window): a key they don't list is
// drift (new-field). `known` lists their keys XenoNet doesn't read. Close an
// object only where the vendor's own types give every key, and list them
// all (docs/design/schemas.md, Making a vendor schema).
export const open = (shape) => z.looseObject(shape);
export const closed = (shape, { known = [] } = {}) => z.looseObject(shape).meta({ "x-xenonet": { closed: true, ...(known.length ? { known } : {}) } });

// One of several kinds of record, told apart by `key` (a line's type): a
// value the schema doesn't list is drift (new-variant), and such values are
// the only ones drift records keep. Kinds XenoNet passes over are listed as
// `skipped`, so they aren't drift.
export const variants = (key, options) => z.discriminatedUnion(key, options).meta({ "x-xenonet": { discriminator: key } });
export const skipped = (key, ...values) => values.map((value) => open({ [key]: z.literal(value) }));

// Other apps' optional fields come and go, and are often null when unset.
export const maybe = (schema) => schema.nullable().optional();
export const num = z.number();
export const str = z.string();
