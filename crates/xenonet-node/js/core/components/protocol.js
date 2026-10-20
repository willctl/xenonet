// core/components/protocol.js in Rust (crates/xenonet-components/src/protocol.rs):
// the wire contracts, which change apart from the components' releases.
import { native } from "../../index.js";
import { settle } from "../../settle.js";

const C = native.compConstants();
export const SERVICE_API = C.serviceApi;
export const CONSOLE_API = C.consoleApi;
export const CAPABILITIES = C.capabilities;

export const negotiate = (local, remote) => settle(native.compNegotiate(local, remote));
