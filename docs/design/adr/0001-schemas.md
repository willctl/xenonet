# 0001. Schemas: zod 4, written out as JSON Schema

Accepted on 2026-10-05, after core-1's spike. `contracts.md` section 1 has
what the schemas are for; `schemas.md` has the layout and the rules.

## Context

- XenoNet's own data (the map, live state, identities, the engine, the doctor,
  themes, `state.json`, and now actors and drift records) has no schema. Its
  shape lives in the code that builds it, and nothing says when it changes.
- The vendor data XenoNet reads (transcripts, rollouts, Claude desktop's and
  Codex's files, the Agent SDK) changes between versions without notice.
- The contracts ask for:
  - JSON Schema draft 2020-12 files, one per schema, each with the `$id`
    `https://xenonet.dev/schemas/<path>/v<major>.json`, with `$ref`s
    between them, generated and committed;
  - a validator in Node, for tests and for reading vendor data leniently
    (unknown keys pass through, and every difference becomes a drift
    record);
  - custom keywords (`x-xenonet`);
  - validation cheap enough for the tailer: tailing and refresh stay within
    10% of the baseline.
- zod 4 is already a dependency: the MCP tools' input schemas use it.
- No new dependency without a reason.

## Options

1. **zod 4 as the source, written out with `z.toJSONSchema`.** The
   definitions are JavaScript modules, and the files are generated from them.
2. **Hand-written JSON Schema, validated with Ajv.** The files are the
   source. Ajv 8 is in `node_modules` already, but only because
   `@modelcontextprotocol/server` uses it: XenoNet would take it on as a
   dependency of its own.
3. **Hand-written JSON Schema, with a validator of our own.** No
   dependency, but a 2020-12 validator (`$ref` resolution, `oneOf`,
   `anyOf`, formats) is real work to write and keep right.

## What the spike found

With zod 4.6.5 (Context7's zod docs, and a script over made-up data):

- **Draft 2020-12** is the default target: every file gets
  `"$schema": "https://json-schema.org/draft/2020-12/schema"`.
- **Ids and refs.** Schemas added to a registry with an `id`, written with
  `z.toJSONSchema(registry, { uri })`, come out one per id, each with
  `$id: uri(id)`. A schema used inside another becomes
  `{"$ref": "<its $id>"}`, so `graph/v1` refers to `session/v1` and
  `identity/v1` instead of copying them.
- **Metadata.** `title`, `description` and custom keys such as `x-xenonet`
  come from `.meta()` on the schema itself, nested ones included. Passing
  the registry as `metadata` too loses the nested ones, so the registry
  holds ids only.
- **Loose objects.** `z.looseObject` keeps unknown keys when parsing and
  writes `additionalProperties: {}`. A plain `z.object` drops them and
  writes `additionalProperties: false`, which would make adding an optional
  field a breaking change for anyone checking against the files.
- **Open enums**, for the contracts' "a value a client doesn't know isn't an
  error": a union of `z.enum([...])` and `z.string()` accepts any string
  and writes the known values in an `anyOf`.
- **Unions.** A discriminated union becomes `oneOf` with a `const` per
  branch. Its errors name the path, and an unknown discriminator value says
  so ("No matching discriminator"), which is what drift needs to tell a
  `new-variant` from a `type-change` or a `missing-field`.
- **The files read back.** `z.fromJSONSchema` turns a file back into a zod
  schema (with local `$ref`s only), so tests can check the files themselves
  against real output without another validator.
- **Parse cost.** 4,000 made-up Claude Code transcript lines in the shapes
  `scripts/bench.mjs` writes, `JSON.parse` and then `safeParse` with a loose,
  discriminated line schema, median of 15 runs, three runs on a busy laptop
  on battery:

| Per line | Time |
|---|---|
| `JSON.parse` only | 0.9 to 1.2 µs |
| and `safeParse` | 1.4 to 3.4 µs |
| and `safeParse` on a schema from `z.compile` | 1.0 to 1.4 µs |
| and `safeParse` on one line in 16 | 0.9 to 1.2 µs |

  Tailing costs about 13 µs a line all told (the benchmark's start reads
  37,000 lines in about 480 ms), so validating every line costs up to 15%
  of tailing, a compiled schema about 2%, and sampling next to nothing.

## Decision

zod 4, with `z.toJSONSchema` writing the files. Option 2 would trade one
library for another and give up the JavaScript definitions that the lenient
reader and drift recording walk anyway; option 3 is a validator to maintain
for no gain.

The rules that keep the files faithful to the code:

- Every object is a `z.looseObject`.
- No transforms, refinements or defaults in a schema: they don't survive
  the trip to JSON Schema. Validate and then convert, in code.
- Ids live in XenoNet's registry (`core/schema/registry.js`); titles,
  descriptions and `x-xenonet` in `.meta()`.
- Hot paths validate a sample, with `z.compile`d schemas.

## What it costs us

- **The files are generated**, so their shape follows zod's writer:
  `oneOf` for discriminated unions, safe-integer bounds on integers,
  `additionalProperties: {}`. A zod upgrade can change them; `npm run
  schemas` shows the difference, and a unit test fails until it's committed.
- **zod checks the data, not the files.** Tests also read the files back
  with `z.fromJSONSchema` and check the demo world's output against those,
  and that every `$ref` resolves. No independent validator (Ajv) checks
  them; adding one as a dev dependency is a question for the owner.
- **Schemas are code.** Anyone outside XenoNet reads the files in `schemas/`;
  only XenoNet imports `core/schema/`.
- **Validation isn't free in the tailer**: up to three times the cost of
  `JSON.parse` uncompiled. Core-3 samples per line type and file and caches
  shapes, as the contracts say, and compiles the line schemas.
- **zod's API is a commitment.** zod 3 to 4 was a large change. The lock
  file pins the version, and the schemas use the plain core of the API
  (objects, unions, enums, records, arrays, `.meta()`).

## Since (core-3)

The vendor readers check data against the schema files in
`schemas/vendor/`, not against zod schemas, compiled or not: a small
walker over the JSON Schema the files hold (`core/schema/lenient.js`)
finds the drift the contracts ask for by kind, and loading the files
costs a millisecond where zod costs about 30 on the server's way up.
zod stays the source of the definitions and what the tests check
XenoNet's own output with. The files now ship with the app.
