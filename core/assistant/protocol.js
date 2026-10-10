// The wire between a remote actor's adapter and the service's private
// endpoint (server/assistant-endpoint.js, client.js): newline-delimited
// JSON over 127.0.0.1 (core/components/rpc.js), at most MAX_LINE a
// message, found through a file in the runtime folder that says the
// endpoint's port and the state folder its grants' credentials are in.
// Its version (ASSISTANT_API) is its own, apart from the service API's and
// the console's.
import path from "node:path";
import { runtimeDir } from "../components/runtime.js";

export const MAX_LINE = 320 << 10;
export const HELLO_KEYS = ["api", "grant_id", "role", "token"];

export const endpointFile = (env = process.env) => path.join(runtimeDir(env), `assistant-${env.XENONET_PORT || 47840}.json`);
