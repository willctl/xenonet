// core/engine/attest.js in Rust (crates/xenonet-engine/src/attest.rs): how
// XenoNet knows who sent a message, an envelope it writes from its own
// checks, and the message's own words quoted beneath XenoNet's lines.
import os from "node:os";
import { native } from "../../index.js";

export const SCHEMA = "message/envelope/v1";

export const attest = ({ sender, method, verdict = null, home = os.homedir(), now = Date.now(), platform = process.platform }) => native.attestEnvelope(sender, method, verdict, home, now, String(platform));

export const verifiedLine = (env, { platform = process.platform } = {}) => native.attestLine(env, String(platform));

export const quoted = (text) => native.attestQuoted(String(text ?? ""));
