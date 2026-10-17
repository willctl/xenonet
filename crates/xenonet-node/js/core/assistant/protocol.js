// core/assistant/protocol.js in Rust (crates/xenonet-assistant/src/
// protocol.rs): the wire between a remote actor's adapter and the
// service's private endpoint, found through a file in the runtime folder.
import { native } from "../../index.js";
import { machine, envOf } from "../../machine.js";

const C = native.asstConstants();

export const MAX_LINE = C.maxLine;
export const HELLO_KEYS = C.helloKeys;

export const endpointFile = (env = process.env) => native.asstEndpointFile(envOf(env), machine());
