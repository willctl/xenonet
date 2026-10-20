// core/actors/you.js in Rust (crates/xenonet-actors/src/you.rs and the
// registry's you_* calls): you at the console, as the window reports you.
// The registry makes you when it's made; createYou gives its you.
import { native } from "../../index.js";

export const YOU_HEARTBEAT_MS = native.actorConstants().youHeartbeatMs;

export const createYou = ({ actors }) => actors.you;
