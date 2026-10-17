// The twin of core/schema/live.js.
// Its zod definitions stay JS: npm run schemas writes schemas/ from them,
// and Rust checks data against the JSON Schema they write
// (crates/xenonet-schema/src/check.rs).
export * from "../../../../../core/schema/live.js";
