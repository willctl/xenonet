// core/instances.js in Rust (crates/xenonet-app/src/instances.rs): the
// instances XenoNet Dev runs as from source, each its own app with its own
// settings, port and single-instance lock.
import { native } from "../index.js";

export const INSTANCES = native.appInstances();

// (process.env holds strings; a variable that isn't one is as if unset.)
const text = (v) => (v == null ? null : String(v));

export function instanceFrom(env = process.env) {
  const answer = native.appInstanceFrom(text(env.XENONET_INSTANCE), text(env.XENONET_DEMO), env.XENONET_PORT == null ? null : String(env.XENONET_PORT));
  if (answer.error) throw new Error(answer.error);
  return answer.ok;
}
