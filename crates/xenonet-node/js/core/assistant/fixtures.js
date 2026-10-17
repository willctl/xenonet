// core/assistant/fixtures.js in Rust (crates/xenonet-assistant/src/
// fixtures.rs): made-up data for remote actors. The Rust gives places as
// pairs; here they're the Maps the JS had.
import { native } from "../../index.js";

const C = native.asstConstants();

export const PHASE_MS = C.phaseMs;
export const FIXTURE = C.fixture;

const withMap = (v) => ({ ...v, places: new Map(v.places) });
const pairs = (v) => ({ ...v, places: v?.places instanceof Map ? [...v.places] : v?.places });

export const phaseAt = (t) => native.asstPhaseAt(Number(t));

export const fixtureView = (t = Date.now()) => withMap(native.asstFixtureView(Number(t)));

export const fixturePolicy = (policy) => withMap(native.asstFixturePolicy(pairs(policy)));

export const fixtureStats = (id) => native.asstFixtureStats(String(id));

export const standalonePolicy = ({ labels = true } = {}) => withMap(native.asstStandalonePolicy(Boolean(labels)));
