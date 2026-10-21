// The twin of core/schema/vendor/spacexai/grok-build.js.
// Its zod definitions stay JS: npm run schemas writes schemas/ from them,
// and Rust checks data against the JSON Schema they write
// (crates/xenonet-schema/src/check.rs).
export * from "../../../../../../../core/schema/vendor/spacexai/grok-build.js";
