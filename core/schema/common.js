// What the schemas are made of.
//
// Every object is loose: zod keeps the keys it doesn't know, and the files
// allow them (additionalProperties), so adding an optional field is never a
// breaking change for anyone checking against a v1. check() with `strict`
// is how tests find keys a schema leaves out (see check.js).
import * as z from "zod/v4";

export { z };

export const obj = (shape) => z.looseObject(shape);

export const ms = z.number().describe("Epoch milliseconds");
export const count = z.int().min(0);
export const str = z.string();
export const nstr = z.string().nullable();

// A value from an open set, as the contracts have it for actors and theme
// tokens: one a client doesn't know is drawn as the default and isn't an
// error. The known values are listed, and any string passes.
export const openEnum = (values) => z.union([z.enum(values), z.string()]);

export const hex = z.string().regex(/^#[0-9a-fA-F]{6}$/).describe("A color, #rrggbb");

// An object that passes another app's data through as it came, so its keys
// aren't XenoNet's to list: check() doesn't count unknown keys in it.
export const passthrough = (schema) => schema.meta({ "x-xenonet": { passthrough: true } });
