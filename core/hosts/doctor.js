// The doctor's check of the hosts as a whole (docs/design/hosts.md), beside
// core/doctor's check of each: whether two hosts answer with the same key.
//
// A XenoNet makes its key once, at random, so two hosts share one only when
// one XenoNet was added twice, or when one machine's state folder was copied
// from the other's (a new Mac set up from an old one's backup, say). The
// console keeps each host's last snapshot under its key, so two such hosts
// share one file in the cache: each writes over the other's, and after a
// restart both show the same machine's map until they answer.
//
// `hosts` is how each host stands, as the hub lists them. Hosts XenoNet Dev
// simulates keep nothing in the cache, and a host that hasn't answered yet
// has no key, so neither counts. With fewer than two hosts there's nothing
// to compare, and no check.

const and = (ids) => (ids.length < 3 ? ids.join(" and ") : `${ids.slice(0, -1).join(", ")} and ${ids.at(-1)}`);

export function keyCheck(hosts) {
  const byKey = new Map();
  for (const h of hosts ?? []) if (h?.key && !h.simulated) (byKey.get(h.key) ?? byKey.set(h.key, []).get(h.key)).push(h.id);
  const shared = [...byKey.values()].filter((ids) => ids.length > 1).map((ids) => ids.sort());
  if (!shared.length) return { status: "ok", detail: `${and([...byKey.values()].flat().sort())} each answer with a key of their own` };
  const [first] = shared;
  const other = first.at(-1);
  return {
    status: "warn",
    detail: `${shared.map((ids) => `${and(ids)} answer with the same key`).join("; ")}, so they keep one snapshot in the cache between them: each writes over the other's, and after a restart they show the same map until they answer`,
    fix: `If ${and(first)} are one XenoNet reached two ways, remove one: xenonet hosts remove ${other}. If they're two machines, one's state folder was copied from the other's: on that one, quit XenoNet, take "self" out of hosts.json in its state folder so it makes a key of its own, and start it again; then accept its new key here: xenonet hosts add ${other} … --replace.`,
  };
}

export function keyChecks(hosts) {
  const keyed = (hosts ?? []).filter((h) => h?.key && !h.simulated);
  if (keyed.length < 2) return [];
  return [{ id: "hosts.keys.own", group: "Hosts", title: "Each host has a key of its own", run: async () => keyCheck(keyed) }];
}
