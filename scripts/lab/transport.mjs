// The transport lab: the checks of phase 5's plan (section 4b) across the
// lab's VMs, which `npm run lab -- check` runs after each VM's own checks.
// Each check needs vm1 and vm2 running. A check that waits on phase 5 code
// says "not in this build yet" when the VMs' XenoNet was built without it,
// and one that needs a network says what the owner runs, since joining
// machines is the owner's (XenoNet refuses it from under an agent). Neither
// counts as a failure.
//
// lab.mjs gives each check `ctx`, which does the work in the VMs (`Ctx`
// below); everything else here is pure, so the tests can check it.
import { randomBytes } from "node:crypto";

// The phase 5 pieces the checks wait on, by the subject of the commit that
// lands each one on main (Conventional Commits, scope net). The lab looks
// for them in the history the VM's XenoNet was built from.
export const LANDS = {
  "link/2": { commit: 9, subject: /^feat\(net\)!?: .*\blink\/2\b/i },
  holders: { commit: 10, subject: /^feat\(net\)!?: .*\bholders?\b/i },
  mailboxes: { commit: 11, subject: /^feat\(net\)!?: .*\bmailbox(es)?\b/i },
  networks: { commit: 13, subject: /^feat\(net\)!?: .*\bseveral networks\b/i },
  "own relays": { commit: "13b", subject: /^feat\(net\)!?: .*\bself-hosted relays\b/i },
};

// The pieces of `needs` the history doesn't have yet.
export function missing(needs, subjects) {
  return needs.filter((n) => !subjects.some((s) => LANDS[n].subject.test(s)));
}

export const waitsOn = (pieces) => `waits on ${pieces.map((p) => `${p} (phase 5 commit ${LANDS[p].commit})`).join(", ")}`;

// What the owner sets up, which an agent can't: XenoNet refuses creating
// and joining networks, and widening them (a relay, a mailbox), from under
// an agent. An agent looks at the same states in XenoNet Dev with the
// `simulate` kinds each check names.
export const OWNER = {
  join: "join vm1 and vm2 to one network with this Mac, as host ids vm1 and vm2: Add a machine in the Mac's XenoNet window, then run the line it shows inside the VM (limactl shell xenonet-vm1), and the same for vm2. Say yes to finding machines on the LAN (mDNS) when it asks",
  relays: (urls) =>
    `add ${urls.join(" and ")} (the lab's own iroh-relays in vm1, with TLS from the lab's CA, which the checks start) as relays of the lab's network in the Mac's XenoNet window, Network settings, then run npm run lab -- check --only ${RELAY_CHECKS.join(",")} again`,
  tcpdump: "install tcpdump in vm1 (limactl shell xenonet-vm1, then sudo apt-get install -y tcpdump), or build the golden copies again, which have it",
  mailbox: "name this Mac as a mailbox of the lab's network in its XenoNet window: Network settings, Mailboxes, Make a mailbox beside the Mac",
  networks: "make a second network, N1, of this Mac and vm1 only, and share atlas-web into it and no other network; the lab's network (N2) keeps vm2 and atlas-api",
};

// The relay checks, which share the lab's relays and the owner's step.
export const RELAY_CHECKS = ["relay", "relay-sealed", "relay-unlisted", "relay-down"];
// The lab's iroh-relays in vm1 (guest.sh's relay step): two the network
// lists, and one it doesn't.
export const RELAYS = { listed: ["a", "b"], unlisted: "elsewhere" };
const RELAY_URLS = ["https://<vm1's lima0 address>:3443", "https://<vm1's lima0 address>:4443"];

export const UDP_PORT = 47850;
export const GROUP = "239.255.47.85";
export const nonce = (what) => `lab-${what}-${randomBytes(6).toString("hex")}`;

// ---------- reading a VM's network ----------

// The networks a VM's XenoNet is in, from `xenonet net status --json`:
// `networks`, a list, once a machine may be in several (phase 5 commit 13),
// and `network` with `hosts` before.
export function networksOf(state) {
  if (Array.isArray(state?.networks)) return state.networks.filter((n) => n?.id);
  if (state?.network?.id) return [{ ...state, ...state.network }];
  return [];
}

// The network both VMs are in, with both of them as members.
export function sharedNetwork(a, b) {
  const ids = new Set(networksOf(b).map((n) => n.id));
  return networksOf(a).find((n) => ids.has(n.id) && ["vm1", "vm2"].every((h) => hostOf(n, h))) ?? null;
}

export const hostOf = (net, id) => (net?.hosts ?? []).find((h) => h?.id === id) ?? null;
export const listed = (net, key) => (Array.isArray(net?.[key]) ? net[key].map((x) => (typeof x === "string" ? x : (x?.url ?? x?.host ?? x?.id))) : []);

// Whether the other machine's link is up, as this VM sees it.
export const online = (state, peer) => networksOf(state).some((n) => hostOf(n, peer)?.online === true);

// An iroh-relay's counter from its metrics (Prometheus text), or null.
export function metric(text, name) {
  const m = String(text ?? "").match(new RegExp(`^relayserver_${name}(?:_total)?(?:\\{[^}]*\\})? (\\d+(?:\\.\\d+)?)$`, "m"));
  return m ? Number(m[1]) : null;
}

// The doctor's row for the relay at `url`, from `xenonet doctor --json`.
export function relayRow(doctorJson, url) {
  let parsed;
  try {
    parsed = JSON.parse(doctorJson);
  } catch {
    return null;
  }
  const bare = String(url).replace(/\/$/, "");
  return (parsed?.results ?? []).find((r) => String(r?.id ?? "").startsWith("network.relay.") && String(r?.title ?? "").includes(bare)) ?? null;
}

const bare = (u) => (typeof u === "string" ? u.replace(/\/$/, "") : u);

// ---------- the firewall rules the checks add ----------

// The addresses of an interface, from `ip -j addr show`.
export function addrsOf(ipJson) {
  let parsed;
  try {
    parsed = JSON.parse(ipJson);
  } catch {
    return [];
  }
  return parsed.flatMap((i) => (i.addr_info ?? []).map((a) => a.local)).filter(Boolean);
}

const set = (addrs) => `{ ${addrs.join(", ")} }`;
const v4 = (addrs) => addrs.filter((a) => !a.includes(":"));
const v6 = (addrs) => addrs.filter((a) => a.includes(":"));

// nft rules a check adds in a table of its own, which `heal` (null) takes
// away and a reboot forgets. `udpTo` drops UDP to and from those addresses
// (the relay check: QUIC's direct path goes, TCP to the relay stays).
// `isolate` drops everything on the lab's LAN interface except to and from
// `allow` (the gap check).
export function rules({ udpTo, isolate, allow = [] }) {
  for (const a of [...(udpTo ?? []), ...allow]) if (!/^[0-9a-f.:]+$/i.test(a)) throw new Error(`not an address: ${a}`);
  if (isolate && !/^[\w.-]+$/.test(isolate)) throw new Error(`not an interface: ${isolate}`);
  const out = [];
  const inn = [];
  if (udpTo?.length) {
    for (const [fam, list] of [["ip", v4(udpTo)], ["ip6", v6(udpTo)]]) {
      if (!list.length) continue;
      out.push(`${fam} daddr ${set(list)} meta l4proto udp drop`);
      inn.push(`${fam} saddr ${set(list)} meta l4proto udp drop`);
    }
  }
  if (isolate) {
    for (const [fam, list] of [["ip", v4(allow)], ["ip6", v6(allow)]]) {
      if (!list.length) continue;
      out.push(`oifname "${isolate}" ${fam} daddr ${set(list)} accept`);
      inn.push(`iifname "${isolate}" ${fam} saddr ${set(list)} accept`);
    }
    out.push(`oifname "${isolate}" drop`);
    inn.push(`iifname "${isolate}" drop`);
  }
  if (!out.length) throw new Error("no rules: give udpTo or isolate");
  const chain = (name, hook, lines) => [`  chain ${name} {`, `    type filter hook ${hook} priority -10; policy accept;`, ...lines.map((l) => `    ${l}`), "  }"];
  return ["table inet xenonet_lab_check", "delete table inet xenonet_lab_check", "table inet xenonet_lab_check {", ...chain("out", "output", out), ...chain("in", "input", inn), "}", ""].join("\n");
}

export const HEAL = "nft delete table inet xenonet_lab_check 2>/dev/null || true";

// ---------- the checks ----------

const other = (vm) => (vm === "vm1" ? "vm2" : "vm1");
const ok = (detail) => ({ status: "ok", detail });
const fail = (detail) => ({ status: "FAIL", detail });
const owner = (detail) => ({ status: "owner", detail });
const answer = (lines, name) => lines.find((l) => l.name === name) ?? null;

async function poll(ctx, ms, fn) {
  const until = Date.now() + ms;
  for (;;) {
    const v = await fn();
    if (v || Date.now() >= until) return v;
    await ctx.sleep(1000);
  }
}

// Both VMs in one network, or what the owner runs.
async function pair(ctx) {
  const a = await ctx.state("vm1");
  const b = await ctx.state("vm2");
  const net = sharedNetwork(a, b);
  return net ? { a, b, net } : { why: OWNER.join };
}

// A stand-in session on `to` that `from`'s stand-in can see, to message.
async function partners(ctx, from, to, project = "atlas-api") {
  const a = ctx.session();
  const b = ctx.session();
  await ctx.standin(to, { session: b, project, calls: [{ name: "whoami" }] });
  const seen = answer(await ctx.standin(from, { session: a, project, calls: [{ name: "peers", until: b, within: 30000 }] }), "peers");
  return { a, b, project, seen: Boolean(seen?.found) };
}

async function end(ctx, ...sessions) {
  for (const [vm, session, project = "atlas-api"] of sessions) await ctx.standin(vm, { session, project, register: false, end: true }).catch(() => {});
}

// A loose message from a stand-in on `from` to one on `to`: whether it
// came, and how long it took by the two VMs' clocks.
async function message(ctx, from, to, { a, b, project, within = 10 }) {
  const text = nonce("message");
  const inbox = ctx.standinLater(to, { session: b, project, register: false, calls: [{ name: "inbox", arguments: { wait_seconds: within } }] });
  await inbox.ready;
  const sent = answer(await ctx.standin(from, { session: a, project, register: false, calls: [{ name: "send", arguments: { to: b, text } }] }), "send");
  const got = answer(await inbox.done, "inbox");
  return { got: Boolean(got?.text?.includes(text)), ms: got && sent ? got.done - sent.at : null, sent: sent?.text ?? "", text };
}

// A handoff, which goes as a kept entry of its machine's feed, and whether
// a session on another VM is offered it.
async function handoff(ctx, vm, goal, project = "atlas-api") {
  const s = ctx.session();
  await ctx.standin(vm, { session: s, project, calls: [{ name: "handoff", arguments: { to: "repo", goal, done: "nothing", next: "nothing: the lab made it" } }], end: true });
}
const offered = async (ctx, vm, session, goal, within) => Boolean(answer(await ctx.standin(vm, { session, project: "atlas-api", register: false, calls: [{ name: "handoff", until: goal, within }] }), "handoff")?.found);

// The lab's listed relays running in vm1, trusted through the lab's CA in
// each VM, the network listing both, and UDP between the VMs dropped, for
// `body`; or what the owner adds. Everything is put back afterwards.
async function withRelays(ctx, body) {
  const p = await pair(ctx);
  if (p.why) return owner(p.why);
  let s;
  try {
    const urls = [];
    for (const name of RELAYS.listed) urls.push(bare(await ctx.relay("vm1", name)));
    await ctx.trustCa(["vm1", "vm2"]);
    const net = sharedNetwork(await ctx.state("vm1"), await ctx.state("vm2")) ?? p.net;
    const have = listed(net, "relays").map(bare);
    const absent = urls.filter((u) => !have.includes(u));
    if (absent.length) return owner(OWNER.relays(absent));
    await ctx.nft("vm1", rules({ udpTo: await ctx.lanAddrs("vm2") }));
    await ctx.nft("vm2", rules({ udpTo: await ctx.lanAddrs("vm1") }));
    const path = await poll(ctx, 60000, async () => hostOf(networksOf(await ctx.state("vm2")).find((n) => n.id === net.id), "vm1")?.path === "relay");
    if (!path) return fail("vm2's path to vm1 didn't move to a relay in 60 s");
    s = await partners(ctx, "vm1", "vm2");
    if (!s.seen) return fail("vm1's stand-in never saw vm2's in peers");
    return await body({ net, urls, s });
  } finally {
    await ctx.nft("vm1", null);
    await ctx.nft("vm2", null);
    await ctx.relayOff("vm1");
    if (s) await end(ctx, ["vm1", s.a], ["vm2", s.b]);
  }
}

// What went into the lab's listed relays, in bytes, by their metrics.
async function relayBytes(ctx) {
  let n = 0;
  for (const name of RELAYS.listed) n += metric(await ctx.relayMetrics("vm1", name), "bytes_recv") ?? 0;
  return n;
}

// Each check: what it shows, the pieces it waits on, what the owner sets
// up for it, the stand-in an agent uses in XenoNet Dev instead, what it
// changes in the VMs (and puts back), and what it reads.
export const CHECKS = [
  {
    id: "lan-udp",
    what: "vm1 and vm2 reach each other over UDP on the lab's LAN",
    needs: [],
    changes: "nothing",
    run: async (ctx) => {
      const bad = [];
      for (const vm of ["vm1", "vm2"]) if (!(await ctx.udp(vm, other(vm)))) bad.push(`${vm} to ${other(vm)}`);
      return bad.length ? fail(`no datagram from ${bad.join(", ")}`) : ok("both ways");
    },
  },
  {
    id: "lan-multicast",
    what: "multicast on the lab's LAN reaches the other VM, as mDNS needs",
    needs: [],
    changes: "nothing",
    run: async (ctx) => ((await ctx.udp("vm1", "vm2", { group: GROUP })) ? ok(`${GROUP} from vm1 to vm2`) : fail(`nothing from ${GROUP} reached vm2`)),
  },
  {
    id: "mac-udp",
    what: "this Mac reaches each VM over UDP on the lab's LAN",
    needs: [],
    changes: "nothing",
    // macOS may ask once whether the app this runs in may reach the local
    // network, in a window: so only when named, by someone at the Mac.
    optIn: "npm run lab -- check --only mac-udp, from your own terminal: macOS may ask once to let it reach the local network",
    run: async (ctx) => {
      const bad = [];
      for (const vm of ["vm1", "vm2"]) if (!(await ctx.udp("mac", vm))) bad.push(vm);
      return bad.length ? fail(`no datagram reached ${bad.join(", ")}`) : ok("vm1 and vm2");
    },
  },
  {
    id: "mdns",
    what: "mDNS finds the other machines, and the path to each is direct",
    needs: ["link/2"],
    owner: OWNER.join,
    standIn: "simulate net-founder",
    changes: "nothing",
    reads: "hosts[].found (how its address was found, \"mdns\" among them) and hosts[].path (\"direct\" or \"relay\")",
    run: async (ctx) => {
      const p = await pair(ctx);
      if (p.why) return owner(p.why);
      const seen = {};
      const good = await poll(ctx, 30000, async () => {
        for (const vm of ["vm1", "vm2"]) {
          const h = hostOf(networksOf(await ctx.state(vm)).find((n) => n.id === p.net.id), other(vm));
          seen[vm] = `${vm} sees ${other(vm)} by ${[].concat(h?.found ?? "nothing").join("+")}, path ${h?.path ?? "none"}`;
          if (![].concat(h?.found ?? []).includes("mdns") || h?.path !== "direct") return false;
        }
        return true;
      });
      return good ? ok(Object.values(seen).join("; ")) : fail(Object.values(seen).join("; "));
    },
  },
  {
    id: "lan-message",
    what: "a message on the LAN arrives in under 1 s",
    needs: ["link/2"],
    owner: OWNER.join,
    standIn: "none: the delivery tests (crates/xenonet-service/tests/net*.rs)",
    changes: "two stand-in sessions, ended afterwards",
    reads: "the engine's peers, send and inbox tools, through a stand-in session in each VM",
    run: async (ctx) => {
      const p = await pair(ctx);
      if (p.why) return owner(p.why);
      const s = await partners(ctx, "vm1", "vm2");
      try {
        if (!s.seen) return fail("vm1's stand-in never saw vm2's in peers");
        const m = await message(ctx, "vm1", "vm2", s);
        if (!m.got) return fail(`vm2's stand-in didn't get it in 10 s (send said: ${m.sent.split("\n")[0]})`);
        return m.ms < 1000 ? ok(`${m.ms} ms, by the VMs' clocks`) : fail(`${m.ms} ms, by the VMs' clocks`);
      } finally {
        await end(ctx, ["vm1", s.a], ["vm2", s.b]);
      }
    },
  },
  {
    id: "relay",
    what: "with UDP between vm1 and vm2 dropped, they link through the lab's own iroh-relay in vm1, its TLS from the lab's CA, and the doctor says so",
    needs: ["link/2", "own relays"],
    owner: `${OWNER.join}; then ${OWNER.relays(RELAY_URLS)}`,
    standIn: "none: the relay tests (crates/xenonet-net/src/relay_tests.rs)",
    changes: "runs the lab's relays in vm1, puts the lab's CA in each VM's relay-ca.pem, and drops UDP between the VMs; the relays stop and the UDP comes back afterwards",
    reads: "the network's relays (URLs), hosts[].path, and the relays' rows in xenonet doctor --json",
    run: (ctx) =>
      withRelays(ctx, async ({ urls, s }) => {
        const doctor = await ctx.doctor("vm2");
        const rows = [];
        for (const url of urls) {
          const row = relayRow(doctor, url);
          if (row?.status !== "ok" || !/a CA you added/.test(row.detail ?? "")) return fail(`vm2's doctor on ${url}: ${row ? `${row.status}, ${row.detail}` : "no row"}`);
          rows.push(`${url} ${(row.detail ?? "").split("; ").find((w) => /ms away/.test(w)) ?? "ok"}`);
        }
        const m = await message(ctx, "vm1", "vm2", s);
        return m.got ? ok(`path relay; a message came in ${m.ms} ms; vm2's doctor: ${rows.join(", ")}`) : fail("the path is the relay, but a message didn't come in 10 s");
      }),
  },
  {
    id: "relay-sealed",
    what: "what crosses the relay is sealed: the text of a message through it is nowhere on vm1's LAN, the relay's own traffic included",
    needs: ["link/2", "own relays"],
    owner: `${OWNER.join}; then ${OWNER.relays(RELAY_URLS)}`,
    standIn: "none: a_relay_sees_who_talks_never_what (crates/xenonet-net/src/relay_tests.rs), which reads what the relay itself sees",
    changes: "as relay, and a packet capture on vm1's lima0 while one message goes, deleted afterwards",
    reads: "a capture of vm1's lima0 (tcpdump, as root), and the relays' byte counters (their metrics, on 127.0.0.1 in vm1)",
    run: (ctx) =>
      withRelays(ctx, async ({ s }) => {
        const before = await relayBytes(ctx);
        if (!(await ctx.capture("vm1", "start"))) return owner(OWNER.tcpdump);
        let m = null;
        let seen = null;
        try {
          m = await message(ctx, "vm1", "vm2", s);
        } finally {
          seen = await ctx.capture("vm1", "stop", m?.text ?? "");
        }
        if (!m.got) return fail("a message through the relay didn't come in 10 s");
        const through = (await relayBytes(ctx)) - before;
        if (!(through > 0)) return fail("the relays' counters didn't move: the message didn't go through them");
        if (!(seen?.bytes > 0)) return fail("the capture on vm1's lima0 caught nothing");
        if (seen.found) return fail(`the message's text crossed vm1's LAN in the clear (${seen.bytes} bytes caught)`);
        return ok(`${through} bytes through the relays, ${seen.bytes} caught on vm1's lima0, the message's text in none`);
      }),
  },
  {
    id: "relay-unlisted",
    what: "a relay the network doesn't list is never used for it: vm2 adds one of its own, and vm1 never contacts it",
    needs: ["link/2", "own relays"],
    owner: `${OWNER.join}; then ${OWNER.relays(RELAY_URLS)}`,
    standIn: "none: a_relay_the_network_doesnt_list_is_never_used (crates/xenonet-net/src/relay_tests.rs)",
    changes: "as relay, and a third relay in vm1 that vm2 adds as its own (xenonet net relay add --mine), taken out afterwards",
    reads: "the unlisted relay's unique_client_keys (its metrics), and messages through stand-ins",
    run: (ctx) =>
      withRelays(ctx, async ({ s }) => {
        const url = await ctx.relay("vm1", RELAYS.unlisted);
        try {
          await ctx.ownRelay("vm2", url, true);
          // vm2's endpoint takes the relay in, then a message goes.
          await ctx.sleep(10000);
          const m = await message(ctx, "vm1", "vm2", s);
          if (!m.got) return fail("with vm2's own relay added, a message didn't come in 10 s");
          const keys = metric(await ctx.relayMetrics("vm1", RELAYS.unlisted), "unique_client_keys");
          if (keys == null) return fail(`couldn't read ${url}'s metrics`);
          if (keys > 1) return fail(`${url} saw ${keys} machines: vm1 contacted a relay its network doesn't list`);
          return ok(`${url} saw ${keys === 1 ? "vm2 alone, whose own it is" : "no machine"}; the message went through the listed relays`);
        } finally {
          await ctx.ownRelay("vm2", url, false);
          await ctx.relayOff("vm1", RELAYS.unlisted);
        }
      }),
  },
  {
    id: "relay-down",
    what: "when vm2's home relay stops, vm2 moves to the other listed relay, messages still come, and vm2's doctor names the one that's down",
    needs: ["link/2", "own relays"],
    owner: `${OWNER.join}; then ${OWNER.relays(RELAY_URLS)}`,
    standIn: "none: a_relay_going_down_moves_relay_only_machines_to_another_listed_one and a_relay_going_down_leaves_direct_paths_and_the_doctor_names_it (crates/xenonet-net/src/relay_tests.rs)",
    changes: "as relay, and stops vm2's home relay; every relay stops afterwards",
    reads: "relaysHere.url (vm2's home relay), hosts[].path, and the relays' rows in xenonet doctor --json",
    run: (ctx) =>
      withRelays(ctx, async ({ net, urls, s }) => {
        const homeOf = async () => bare((await ctx.state("vm2"))?.relaysHere?.url);
        const home = await homeOf();
        const name = RELAYS.listed[urls.indexOf(home)];
        if (!name) return fail(`vm2's home relay is ${home ?? "none"}, not one of the lab's (${urls.join(", ")})`);
        const left = urls.find((u) => u !== home);
        await ctx.relayOff("vm1", name);
        if (!(await poll(ctx, 60000, async () => (await homeOf()) === left))) return fail(`vm2's home didn't move from ${home} to ${left} in 60 s`);
        if (!(await poll(ctx, 60000, async () => hostOf(networksOf(await ctx.state("vm2")).find((n) => n.id === net.id), "vm1")?.path === "relay"))) return fail("vm2's path to vm1 isn't through a relay after the move");
        const m = await message(ctx, "vm1", "vm2", s);
        if (!m.got) return fail(`with ${home} down, a message didn't come in 10 s`);
        const row = relayRow(await ctx.doctor("vm2"), home);
        if (!row || row.status === "ok" || !(row.detail ?? "").includes(home)) return fail(`vm2's doctor doesn't name ${home} as down: ${row ? row.detail : "no row"}`);
        return ok(`home moved to ${left}; a message came in ${m.ms} ms; vm2's doctor: ${row.detail}`);
      }),
  },
  {
    id: "mailbox",
    what: "after down vm2, a mailbox delivers to vm2 once it's back, with the sender offline",
    needs: ["link/2", "mailboxes"],
    owner: `${OWNER.join}; then ${OWNER.mailbox}`,
    standIn: "simulate net-away (what's held for a machine that's away, the console's side only)",
    changes: "stops vm2 and vm1's XenoNet, and starts both again",
    reads: "the network's mailboxes (host ids)",
    run: async (ctx) => {
      const p = await pair(ctx);
      if (p.why) return owner(p.why);
      if (!listed(p.net, "mailboxes").some((h) => h !== "vm1" && h !== "vm2")) return owner(OWNER.mailbox);
      const s = await partners(ctx, "vm1", "vm2");
      const text = nonce("mailbox");
      try {
        if (!s.seen) return fail("vm1's stand-in never saw vm2's in peers");
        await ctx.down("vm2");
        await ctx.standin("vm1", { session: s.a, project: s.project, register: false, calls: [{ name: "send", arguments: { to: s.b, text } }] });
        await ctx.service("vm1", "stop");
        await ctx.up("vm2");
        const got = answer(await ctx.standin("vm2", { session: s.b, project: s.project, calls: [{ name: "inbox", arguments: { wait_seconds: 60 } }] }), "inbox");
        return got?.text?.includes(text) ? ok("vm2 got it with vm1 offline") : fail("vm2 didn't get it in 60 s with vm1 offline");
      } finally {
        await ctx.service("vm1", "start");
        await ctx.up("vm2");
        await end(ctx, ["vm1", s.a], ["vm2", s.b]);
      }
    },
  },
  {
    id: "gap",
    what: "after a partition, vm2 fills its gap from vm1",
    needs: ["link/2", "holders"],
    owner: OWNER.join,
    standIn: "none: the turmoil tests (crates/xenonet-net/src/sim_tests.rs)",
    changes: "cuts vm2 off the lab's LAN, then lets vm1 alone back in, then puts it back",
    reads: "the engine's handoff tool, through stand-in sessions",
    run: async (ctx) => {
      const p = await pair(ctx);
      if (p.why) return owner(p.why);
      const b = ctx.session();
      const goal = nonce("gap");
      try {
        await ctx.standin("vm2", { session: b, project: "atlas-api", calls: [{ name: "whoami" }] });
        await ctx.nft("vm2", rules({ isolate: ctx.lanDev }));
        for (const i of [1, 2, 3]) await handoff(ctx, "vm1", `${goal} ${i}`);
        // Back on the LAN with vm1 only, so the gap can fill from nowhere
        // else.
        await ctx.nft("vm2", rules({ isolate: ctx.lanDev, allow: await ctx.lanAddrs("vm1") }));
        const all = await offered(ctx, "vm2", b, `${goal} 3`, 60000);
        const first = all && (await offered(ctx, "vm2", b, `${goal} 1`, 0)) && (await offered(ctx, "vm2", b, `${goal} 2`, 0));
        return first ? ok("all three handoffs came from vm1") : fail(all ? "the last came, an earlier one didn't" : "the handoffs didn't come in 60 s");
      } finally {
        await ctx.nft("vm2", null);
        await end(ctx, ["vm2", b]);
      }
    },
  },
  {
    id: "skew",
    what: "vm2's clock 3 min ahead is ordered by it; 6 min ahead is accepted, clamped, and the doctor shows the skew",
    needs: ["link/2"],
    owner: OWNER.join,
    standIn: "none in XenoNet Dev: the skew is measured on a real hello, so it takes two machines",
    changes: "sets vm2's clock ahead with NTP off, then puts both back",
    reads: "hosts[].skewMs (the peer's clock ahead of this one's, measured on the hello), self.hlcAheadMs (this machine's HLC ahead of its clock) and xenonet doctor --json",
    run: async (ctx) => {
      const p = await pair(ctx);
      if (p.why) return owner(p.why);
      const b = ctx.session();
      const notes = [];
      try {
        await ctx.standin("vm1", { session: b, project: "atlas-api", calls: [{ name: "whoami" }] });
        for (const min of [3, 6]) {
          await ctx.clock("vm2", min * 60000);
          const goal = nonce(`skew${min}`);
          await handoff(ctx, "vm2", goal);
          if (!(await offered(ctx, "vm1", b, goal, 60000))) return fail(`a handoff from vm2 ${min} min ahead didn't come`);
          const st = await ctx.state("vm1");
          const skew = hostOf(networksOf(st).find((n) => n.id === p.net.id), "vm2")?.skewMs;
          const ahead = st?.self?.hlcAheadMs;
          if (typeof skew !== "number" || Math.abs(Math.abs(skew) - min * 60000) > 60000) return fail(`vm1 shows vm2's skew as ${skew ?? "nothing"} at ${min} min`);
          if (min === 3 && !(ahead >= 150000)) return fail(`vm1's HLC didn't follow vm2's 3 min ahead (${ahead ?? "?"} ms)`);
          if (min === 6 && !(ahead <= 305000)) return fail(`vm1's HLC went past 5 min ahead (${ahead ?? "?"} ms)`);
          if (min === 6 && !/vm2/.test(await ctx.doctor("vm1"))) return fail("vm1's doctor doesn't name vm2's skew");
          notes.push(`${min} min: accepted, skew ${Math.round(skew / 1000)} s, HLC ${Math.round(ahead / 1000)} s ahead`);
        }
        return ok(notes.join("; "));
      } finally {
        await ctx.clock("vm2", null);
        await end(ctx, ["vm1", b]);
      }
    },
  },
  {
    id: "networks",
    what: "vm1 in N1 and N2, vm2 in N2 only: vm2 sees nothing of N1",
    needs: ["link/2", "networks"],
    owner: `${OWNER.join}; then ${OWNER.networks}`,
    standIn: "none: the several-networks tests (crates/xenonet-service/tests/net*.rs)",
    changes: "a stand-in session in each VM, ended afterwards",
    reads: "networks[] (id, hosts) from xenonet net status --json, and the engine's tools through stand-ins",
    run: async (ctx) => {
      const a = await ctx.state("vm1");
      const b = await ctx.state("vm2");
      const n2 = sharedNetwork(a, b);
      const theirs = new Set(networksOf(b).map((n) => n.id));
      const n1 = networksOf(a).find((n) => !theirs.has(n.id));
      if (!n2) return owner(OWNER.join);
      if (!n1) return owner(OWNER.networks);
      const s1 = ctx.session();
      const s2 = ctx.session();
      const text = nonce("n1");
      try {
        await ctx.standin("vm2", { session: s2, project: "atlas-api", calls: [{ name: "whoami" }] });
        const inbox = ctx.standinLater("vm2", { session: s2, project: "atlas-api", register: false, calls: [{ name: "inbox", arguments: { wait_seconds: 10 } }] });
        await inbox.ready;
        await ctx.standin("vm1", { session: s1, project: "atlas-web", calls: [{ name: "claim", arguments: { paths: ["src/"], note: text } }, { name: "send", arguments: { to: "all", text } }] });
        const got = answer(await inbox.done, "inbox")?.text ?? "";
        const seen = JSON.stringify(await ctx.state("vm2")) + (answer(await ctx.standin("vm2", { session: s2, project: "atlas-api", register: false, calls: [{ name: "peers" }] }), "peers")?.text ?? "");
        const leaks = [[text, "N1's message or claim"], [s1, "N1's session"], [n1.id, "N1's id"]].filter(([k]) => got.includes(k) || seen.includes(k)).map(([, what]) => what);
        return leaks.length ? fail(`vm2 sees ${leaks.join(", ")}`) : ok(`nothing of N1 (${n1.name ?? n1.id}) on vm2 after 10 s`);
      } finally {
        await end(ctx, ["vm1", s1, "atlas-web"], ["vm2", s2]);
      }
    },
  },
  {
    id: "restore",
    what: "a restored state dir starts a new incarnation and doesn't freeze",
    needs: [],
    owner: OWNER.join,
    standIn: "simulate net-frozen (what a freeze looks like)",
    changes: "copies vm2's XenoNet state dir aside, writes past it, puts the copy back, and drops the copy",
    reads: "the engine's handoff tool through stand-ins, and frozen[] from vm1's xenonet net status --json",
    run: async (ctx) => {
      const p = await pair(ctx);
      if (p.why) return owner(p.why);
      const b = ctx.session();
      const first = nonce("before");
      const second = nonce("after");
      const linked = () => poll(ctx, 60000, async () => online(await ctx.state("vm2"), "vm1") || online(await ctx.state("vm1"), "vm2"));
      try {
        await ctx.standin("vm1", { session: b, project: "atlas-api", calls: [{ name: "whoami" }] });
        await ctx.service("vm2", "stop");
        await ctx.snapshot("vm2", "save");
        await ctx.service("vm2", "start");
        await linked();
        await handoff(ctx, "vm2", first);
        if (!(await offered(ctx, "vm1", b, first, 60000))) return fail("a handoff from vm2 didn't come before the restore");
        await ctx.service("vm2", "stop");
        await ctx.snapshot("vm2", "restore");
        await ctx.service("vm2", "start");
        await linked();
        await handoff(ctx, "vm2", second);
        const came = await offered(ctx, "vm1", b, second, 60000);
        const frozen = networksOf(await ctx.state("vm1")).flatMap((n) => n.frozen ?? []).filter((f) => f?.host === "vm2");
        const said = /new incarnation/.test(await ctx.logs("vm2")) ? "; vm2's log says it wrote from a new incarnation" : "";
        if (frozen.length) return fail("vm1 froze vm2's feed after the restore");
        return came ? ok(`vm2's next handoff came, nothing frozen${said}`) : fail(`vm2's handoff after the restore didn't come in 60 s${said}`);
      } finally {
        await ctx.snapshot("vm2", "drop");
        await ctx.service("vm2", "start");
        await end(ctx, ["vm1", b]);
      }
    },
  },
];

// One check: "skip" without both VMs (saying what this checkout lacks,
// which a VM built from it would too), "wait" when the VMs' build lacks a
// piece, then the check's own answer ("owner" when the owner's setup is
// missing). A check that throws fails with the error.
export async function runCheck(check, ctx) {
  if (!ctx.both) {
    const here = missing(check.needs, await ctx.subjects("here"));
    return { status: "skip", detail: `needs vm1 and vm2 running${here.length ? `; this checkout ${waitsOn(here)}` : ""}` };
  }
  const lacking = new Set();
  for (const vm of ["vm1", "vm2"]) for (const piece of missing(check.needs, await ctx.subjects(vm))) lacking.add(piece);
  if (lacking.size) return { status: "wait", detail: `not in this build yet: ${waitsOn([...lacking])}` };
  try {
    return await check.run(ctx);
  } catch (err) {
    return fail(err?.message ?? String(err));
  }
}
