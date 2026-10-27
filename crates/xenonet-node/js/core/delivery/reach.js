// core/delivery/reach.js in Rust (crates/xenonet-engine/src/delivery/reach.rs):
// what this machine lets a sender do to its sessions and actors, decided
// where the message arrives.
import { native } from "../../index.js";

export const ACTIONS = ["message", "wake", "start"];

export const senderOf = (id) => native.reachSender(id ?? null);

export const decide = (action, sender, target, settings = {}) => native.reachDecide(String(action), sender ?? null, target ?? null, settings ?? null);
