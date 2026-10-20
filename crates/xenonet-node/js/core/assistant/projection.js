// core/assistant/projection.js in Rust (crates/xenonet-assistant/src/
// projection.rs): what a remote actor may see of XenoNet's state. A
// view's and a policy's places go to the Rust as pairs.
import { native } from "../../index.js";

const C = native.asstConstants();

export const LABEL_MAX = C.labelMax;
export const LIVE_MS = C.liveMs;
export const WINDOW_MS = C.windowMs;
export const LOOKBACK_MAX_MS = C.lookbackMaxMs;
export const SOURCE_NAMES = C.sourceNames;

const plain = (v) => (v?.places instanceof Map ? { ...v, places: [...v.places] } : v);

export const cleanLabel = (text) => native.asstCleanLabel(text);

export const sessionStatus = (live, sourceState = "ready") => native.asstSessionStatus(live, String(sourceState));

export const stallReason = (reasons = []) => native.asstStallReason(reasons);

export const visibleSessions = (view, policy, now) => native.asstVisibleSessions(plain(view), plain(policy), Number(now));

export const sessionSummary = (v, policy) => native.asstSessionSummary(v, plain(policy));

export const emptyCounts = () => ({ working: 0, waiting: 0, idle: 0, completed: 0, stuck: 0, unknown: 0 });

export const freshnessOf = (view, now, names = SOURCE_NAMES) => native.asstFreshnessOf(plain(view), Number(now), names.map(String));

export const sourcesData = (view) => native.asstSourcesData(plain(view));

export const attentionItems = (view, policy, now, { idOf }) => native.asstAttentionItems(plain(view), plain(policy), Number(now), (parts) => String(idOf(parts)));

export const actorSummaries = (view, policy, now, { projectId = null } = {}) => native.asstActorSummaries(plain(view), plain(policy), Number(now), projectId);

export const statsData = (raw, now) => native.asstStatsData(raw, Number(now));
