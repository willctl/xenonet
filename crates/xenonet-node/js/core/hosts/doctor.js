// core/hosts/doctor.js in Rust (crates/xenonet-hosts/src/doctor.rs): the
// doctor's check of the hosts as a whole, whether two hosts answer with the
// same key.
import { native } from "../../index.js";

const plain = (hosts) => (hosts ?? []).map((h) => (h && typeof h === "object" ? { id: h.id, key: h.key, simulated: h.simulated } : h));

export const keyCheck = (hosts) => native.hostsKeyCheck(plain(hosts));

export function keyChecks(hosts) {
  const keyed = native.hostsKeyed(plain(hosts));
  if (keyed.length < 2) return [];
  return [{ id: "hosts.keys.own", group: "Hosts", title: "Each host has a key of its own", run: async () => keyCheck(keyed) }];
}
