// core/actors/remote-presence.js in Rust (crates/xenonet-actors/src/remote_presence.rs):
// what a remote actor's reads say of it, put to the actors' registry. What's
// kept and when each word is taken are Rust's; the registry and onRead are
// called as the JS called them.
import { native } from "../../index.js";

export const remoteInstance = (connectionId) => native.remoteInstance(connectionId);

export function createRemotePresence({ actors, now = () => actors.now(), onRead = null }) {
  const core = new native.RemotePresenceCore();
  const deps = { actors, onRead: onRead ? (read) => onRead(read) : null };
  return {
    read(actorId, connectionId, { targets = [], reach = targets, at = now() } = {}) {
      core.read(deps, actorId, connectionId, [...targets], [...reach], at, !!onRead);
    },
  };
}
