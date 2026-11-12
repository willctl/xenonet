// The VM lab (scripts/lab): the hash it gives each VM for sudo, the
// settings in its templates, the made-up repos it puts in each VM, and the
// transport checks, run against a made-up lab. Nothing here starts a VM.
import { bash, runBash } from "./harness/bash.js";
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { BASES, VMS, LAB_INSTANCES, HOST_ID, LAN_DEV, labPaths, envFileText, guestScript, shq, sha512crypt, randomSalt, newPassword } from "../scripts/lab/config.mjs";
import { CHECKS, LANDS, OWNER, RELAY_CHECKS, metric, missing, relayRow, runCheck, rules, addrsOf, networksOf, sharedNetwork } from "../scripts/lab/transport.mjs";
import { REPOS, reposFor, reposScript } from "../scripts/lab/fixtures.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const lab = (...p) => path.join(root, "scripts", "lab", ...p);
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-lab-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

const H = "/Users/me";

test("SHA-512 crypt matches the specification's results and OpenSSL's", () => {
  assert.equal(sha512crypt("Hello world!", "saltstring"), "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1");
  // Salts stop at 16 characters; rounds are written out when they're given.
  assert.equal(sha512crypt("Hello world!", "saltstringsaltstring"), "$6$saltstringsaltst$e.3mR68CqZEpesEX1HlFZT6sEanSOjM/b5UoDyDo00a8syek2cJldMjrbtKP86.FJvzluVR7nc3DNzelAwTxj.");
  assert.equal(sha512crypt("Hello world!", "saltstringsaltstring", { rounds: 10000 }), "$6$rounds=10000$saltstringsaltst$OW1/O6BYHV6BcXZu8QVeXbDWra3Oeqh0sbHbbMCVNSnCM/UrjmM0Dp8vOuZeHBy/YTBmSK6H9qs/y3RnOaw5v.");
  assert.equal(sha512crypt("we have a short salt string but not a short password", "short"), "$6$short$qmfj2meTBr5G2EAGIJ4vjX7RpefsD4JzpEyTAeEUJdzdxlBS6pe8gdMHm5zFftaFSj/2p2bjBwyVS9ZhWpLZt.");
  // A password longer than two SHA-512 blocks.
  assert.equal(sha512crypt("p".repeat(130), "abcdefgh12345678"), "$6$abcdefgh12345678$vNtNwbew.y5XXSX3MuSD6Bzw5OaW46zZW.yvujFekbkZLlqEA3NWiOwDtxNy.anEkNf3cn5a5E3E.KXIYTUGU0");
  assert.match(randomSalt(), /^[./0-9A-Za-z]{16}$/);
  assert.match(newPassword(), /^[\w-]{32}$/);
  assert.notEqual(newPassword(), newPassword());
});

test("the VMs: host ids, one SSH port each, golden copies that exist, names teardown knows", () => {
  const ports = Object.values(VMS).map((v) => v.sshPort);
  assert.equal(new Set(ports).size, ports.length);
  for (const [id, vm] of Object.entries(VMS)) {
    assert.match(id, HOST_ID);
    assert.ok(BASES[vm.base], `${id}'s golden copy`);
    assert.ok(LAB_INSTANCES.includes(vm.instance));
  }
  for (const b of Object.values(BASES)) {
    assert.ok(LAB_INSTANCES.includes(b.instance));
    assert.ok(fs.existsSync(lab("lima", b.template)), b.template);
  }
  // Only one VM asks Anthropic for plan usage, and the two disagree on the time.
  assert.equal(Object.values(VMS).filter((v) => v.env.XENONET_CLAUDE_USAGE !== "off").length, 1);
  assert.notEqual(VMS.vm1.timezone, VMS.vm2.timezone);
  for (const vm of Object.values(VMS)) assert.match(vm.timezone, /^[A-Za-z_]+(\/[A-Za-z0-9_+-]+)*$/);
});

test("each VM's password goes in its Lima folder, or where LIMA_HOME says", () => {
  assert.equal(labPaths({ env: {}, home: H }).password("xenonet-vm1"), path.join(H, ".lima", "xenonet-vm1", "xenonet-lab-password"));
  assert.equal(labPaths({ env: { LIMA_HOME: "/m" }, home: H }).instanceDir("xenonet-vm2"), path.join(path.resolve("/m"), "xenonet-vm2"));
});

test("a VM's settings file and guest.sh's values are quoted for the shell", { skip: !bash && "Bash is required" }, () => {
  assert.equal(envFileText(VMS.vm2.env), "XENONET_CLAUDE_USAGE=off\n");
  assert.equal(envFileText({}), "");
  assert.throws(() => envFileText({ X: "a\nB=c" }), /bad setting/);
  assert.throws(() => envFileText({ "lower": "x" }), /bad setting/);
  const script = guestScript("vm", { HOST_ID: "vm1", LINE: `it's "quoted" $HOME` }, "echo body\n");
  assert.equal(script, `STEP='vm'\nHOST_ID='vm1'\nLINE='it'\\''s "quoted" $HOME'\necho body\n`);
  const r = runBash( ["-c", `${script.replace("echo body", 'printf "%s|%s" "$STEP" "$LINE"')}`], { encoding: "utf8" });
  assert.equal(r.stdout, `vm|it's "quoted" $HOME`);
  assert.throws(() => guestScript("rm -rf", {}, ""), /bad step/);
  assert.throws(() => guestScript("vm", { "A;B": 1 }, ""), /bad variable/);
  assert.equal(shq("a'b"), `'a'\\''b'`);
});

test("versions.json pins Node with its checksum, Claude Code, Codex, and iroh-relay at the workspace's iroh", () => {
  const v = JSON.parse(fs.readFileSync(lab("versions.json"), "utf8"));
  const iroh = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8").match(/^iroh = \{ version = "([^"]+)"/m)?.[1];
  assert.ok(iroh, "Cargo.toml pins iroh");
  assert.equal(v["iroh-relay"], iroh, "the relay check's iroh-relay is the iroh XenoNet uses");
  assert.match(v.node.version, /^22\.\d+\.\d+$/);
  assert.match(v.node.sha256["linux-arm64"], /^[0-9a-f]{64}$/);
  assert.match(v.claude, /^\d+\.\d+\.\d+$/);
  assert.match(v.codex, /^\d+\.\d+\.\d+$/);
});

test("the templates: plain mode, no mounts, a made-up user, no agent forwarding, the loopback blocked", () => {
  const shared = fs.readFileSync(lab("lima", "lab.yaml"), "utf8");
  for (const line of ["plain: true", "mounts: []", "propagateProxyEnv: false", "  name: will", "  comment: Will", "  forwardAgent: false", "  loadDotSSHPubKeys: false", "  overVsock: false"]) {
    assert.ok(shared.split("\n").includes(line), line);
  }
  // New TCP connections and UDP to the Mac's loopback are dropped by flags,
  // not conntrack's state, which would drop replies on an SSH connection
  // that was open before the rules loaded.
  assert.match(shared, /ip daddr 192\.168\.5\.2 tcp flags & \(syn \| ack\) == syn drop/);
  assert.match(shared, /ip daddr 192\.168\.5\.2 meta l4proto udp drop/);
  assert.ok(!/ct state/.test(shared));
  assert.match(shared, /ssh_deletekeys: false/);
  // A second interface on Apple's shared NAT, for UDP and multicast between
  // the VMs and the Mac.
  assert.match(shared, /^networks:\n- vzNAT: true$/m);
  assert.equal(LAN_DEV, "lima0");
  assert.match(shared, /PasswordAuthentication no/);
  assert.match(shared, /will ALL=\(ALL\) ALL\n/);
  assert.match(shared, /loginctl enable-linger will/);
  // Lima ignores its own timezone setting in plain mode, so a parameter carries it.
  assert.ok(!/^timezone:/m.test(shared));
  assert.match(shared, /^ {2}labTimezone: UTC$/m);
  assert.match(shared, /timedatectl set-timezone "\$\{PARAM_labTimezone:-UTC\}"/);
  for (const b of Object.values(BASES)) {
    const t = fs.readFileSync(lab("lima", b.template), "utf8");
    assert.match(t, /^- template:_images\/(ubuntu-24\.04|fedora-44)$/m, `${b.template} uses the image alone, without Lima's home-folder mount`);
    assert.match(t, /^- \.\/lab\.yaml$/m);
  }
});

test("the made-up repos: atlas-api on every host with one remote, the rest on one host, Linux edge cases", { skip: !bash && "Bash is required" }, () => {
  const paths = (host) => reposFor(host).map((r) => r.path);
  assert.deepEqual(paths("vm1"), ["atlas-api", "atlas-web", "labs", "labs/geofence", "Café Zones", "atlas-kit"]);
  assert.deepEqual(paths("vm2"), ["atlas-api", "ledger", "docs", "Café Zones", "atlas-kit"]);
  assert.deepEqual(paths("mac1"), ["atlas-api", "pathfinder"]);
  assert.throws(() => reposScript("vm9"), /no repos/);
  // The demo world's remotes, and only atlas-api's on more than one host.
  const remotes = Object.fromEntries(REPOS.filter((r) => r.remote).map((r) => [r.path, r.remote]));
  assert.equal(remotes["atlas-api"], "https://github.com/atlas-co/atlas-api.git");
  assert.equal(remotes.ledger, "https://github.com/nova-dev/ledger.git");
  assert.equal(remotes["labs/geofence"], "https://github.com/orbitworks/geofence.git");
  assert.deepEqual(REPOS.filter((r) => r.remote && r.hosts.length > 1).map((r) => r.path), ["atlas-api"]);
  for (const r of REPOS) for (const c of r.commits ?? []) assert.match(c[0], /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/, `${r.path}'s dates are fixed`);
  assert.equal(runBash( ["-n"], { input: reposScript("vm1") }).status, 0);
});


// ---------- the transport checks ----------

// Subjects of commits on main before link/2, and the ones the plan names
// for the pieces the checks wait on.
const BEFORE = ["feat(net): the xenonet-net/2 envelope", "feat(log): relays, mailboxes and rekey chain ops; AddressRecord", "feat(net): Dial, the iroh endpoint and admission", "test(net): the turmoil harness"];
const PLANNED = ["feat(net): link/2: hello, kept/loose/live streams, Want/Ack", "feat(net): fetch from any holder", "feat(net): mailboxes and Held receipts", "feat(net)!: several networks per machine", "feat(net): self-hosted relays: your own CA, listed relays only, doctor rows"];

test("a transport check waits on the phase 5 commits it needs, found by their subjects", () => {
  assert.deepEqual(missing(Object.keys(LANDS), BEFORE), ["link/2", "holders", "mailboxes", "networks", "own relays"]);
  assert.deepEqual(missing(Object.keys(LANDS), [...BEFORE, ...PLANNED]), []);
  assert.deepEqual(missing(["link/2"], ["docs: link/2 in the plan", "feat(net): link/2"]), [], "a feat(net) commit lands it");
  assert.deepEqual(missing(["mailboxes"], ["docs(net): mailboxes"]), ["mailboxes"], "only a feat(net) commit lands a piece");
  assert.deepEqual(Object.values(LANDS).map((l) => l.commit), [9, 10, 11, 13, "13b"]);
});

test("the transport checks: the plan's eight, the LAN's three and the own relay's four, each saying what it needs and changes", () => {
  const ids = CHECKS.map((c) => c.id);
  assert.deepEqual(ids, ["lan-udp", "lan-multicast", "mac-udp", "mdns", "lan-message", ...RELAY_CHECKS, "mailbox", "gap", "skew", "networks", "restore"]);
  for (const c of CHECKS) {
    for (const n of c.needs) assert.ok(LANDS[n], `${c.id} needs ${n}`);
    assert.ok(c.what && c.changes && typeof c.run === "function", c.id);
    // Every check past the LAN's own needs a network, which is the owner's
    // to make, and names what an agent looks at instead.
    if (!/^(lan-udp|lan-multicast|mac-udp)$/.test(c.id)) assert.ok(c.owner?.includes(OWNER.join) && c.standIn, c.id);
  }
  const by = Object.fromEntries(CHECKS.map((c) => [c.id, c.needs]));
  assert.deepEqual(by.mailbox, ["link/2", "mailboxes"]);
  assert.deepEqual(by.gap, ["link/2", "holders"]);
  assert.deepEqual(by.networks, ["link/2", "networks"]);
  assert.deepEqual(by.restore, [], "a restore's new incarnation is in this build");
  for (const id of RELAY_CHECKS) assert.deepEqual(by[id], ["link/2", "own relays"], id);
  // Only the Mac's own datagram may open a macOS window, so it runs only
  // when named.
  assert.deepEqual(CHECKS.filter((c) => c.optIn).map((c) => c.id), ["mac-udp"]);
});

// A made-up lab that answers each check as it should: two VMs in one
// network (and vm1 in a second one), stand-ins whose messages and handoffs
// arrive, and the firewall, clock and relay the checks change.
function fakeLab({ subjects = [...BEFORE, ...PLANNED], network = true, relays = true } = {}) {
  const sent = new Map();
  const goals = new Set();
  const lab = { nft: {}, clock: 0, off: new Set(), bytes: 0, own: [], ca: [] };
  const urls = { a: "https://192.168.64.2:3443", b: "https://192.168.64.2:4443", elsewhere: "https://192.168.64.2:5443" };
  const home = () => [urls.a, urls.b].find((u) => !lab.off.has(u)) ?? null;
  const host = (id, me) => ({ id, online: true, found: ["mdns"], path: lab.nft[me] ? "relay" : "direct", skewMs: id === "vm2" ? lab.clock : 0 });
  const n2 = (me) => ({ id: "net-2", name: "lab", hosts: [host("mac", me), host("vm1", me), host("vm2", me)], relays: relays ? [`${urls.a}/`, urls.b] : [], mailboxes: ["mac"], frozen: [] });
  const n1 = (me) => ({ id: "net-1", name: "N1", hosts: [host("mac", me), host("vm1", me)], relays: [], mailboxes: [], frozen: [] });
  const standin = async (vm, o) =>
    (o.calls ?? []).map((c) => {
      const at = Date.now();
      let text = "";
      if (c.name === "send") sent.set(c.arguments.to, c.arguments.text);
      if (c.name === "handoff" && c.arguments?.goal) goals.add(c.arguments.goal);
      if (c.name === "inbox") text = sent.get(o.session) ?? "";
      if (c.name === "handoff" && c.until) text = goals.has(c.until) ? c.until : "";
      if (c.name === "peers") text = c.until ?? "";
      return { name: c.name, at, done: at + 5, error: false, found: c.until ? text.includes(c.until) : undefined, text };
    });
  return {
    lab,
    both: true,
    lanDev: "lima0",
    sleep: async () => {},
    session: () => `s-${Math.random().toString(16).slice(2)}`,
    subjects: async () => subjects,
    state: async (vm) => (network ? { networks: vm === "vm1" ? [n1(vm), n2(vm)] : [n2(vm)], self: { id: vm, hlcAheadMs: Math.min(lab.clock, 300000) }, relaysHere: { url: `${home()}/` } } : { network: null, hosts: [] }),
    standin,
    standinLater: (vm, o) => ({ ready: Promise.resolve(), done: new Promise((r) => setImmediate(() => r(standin(vm, o)))) }),
    lanAddrs: async (vm) => (vm === "vm1" ? ["192.168.64.2", "fe80::1"] : ["192.168.64.3"]),
    nft: async (vm, text) => (lab.nft[vm] = text),
    clock: async (vm, offset) => (lab.clock = offset ?? 0),
    down: async () => {},
    up: async () => {},
    service: async () => {},
    snapshot: async () => {},
    relay: async (vm, name) => (lab.off.delete(urls[name]), urls[name]),
    relayOff: async (vm, name) => {
      for (const [n, u] of Object.entries(urls)) if (!name || n === name) lab.off.add(u);
    },
    relayMetrics: async (vm, name) => `# HELP\nrelayserver_bytes_recv_total ${(lab.bytes += 100)}\nrelayserver_unique_client_keys_total ${name === "elsewhere" ? lab.own.length : 2}\n`,
    trustCa: async (vms) => lab.ca.push(...vms),
    ownRelay: async (vm, u, add) => (lab.own = add ? [u] : []),
    capture: async (vm, verb) => (verb === "start" ? true : { bytes: 4096, found: false }),
    doctor: async () =>
      JSON.stringify({
        results: [
          { id: "network.clock", title: "This computer's clock is close", detail: "vm2 is 6 min ahead" },
          ...[urls.a, urls.b].map((u) => ({ id: `network.relay.${u.slice(8)}`, title: `The relay ${u} answers`, ...(lab.off.has(u) ? { status: "warn", detail: `${u} doesn't answer at 192.168.64.2: refused` } : { status: "ok", detail: "listed by lab; 2 ms away; TLS valid, a certificate from a CA you added (relay-ca.pem); iroh-relay 1.4.1" }) })),
        ],
      }),
    logs: async () => "vm1 holds more of this machine's feed than it has: its store was restored, so it writes from a new incarnation",
    udp: async () => true,
  };
}

test("every transport check passes on a lab that behaves, and puts back what it changed", async () => {
  for (const c of CHECKS) {
    const ctx = fakeLab();
    const r = await runCheck(c, ctx);
    assert.equal(r.status, "ok", `${c.id}: ${r.detail}`);
    assert.deepEqual(Object.values(ctx.lab.nft).filter(Boolean), [], `${c.id} heals the firewall`);
    assert.equal(ctx.lab.clock, 0, `${c.id} puts the clock back`);
  }
});

test("a transport check skips without both VMs, waits on missing code, and asks the owner for a network", async () => {
  const restore = CHECKS.find((c) => c.id === "restore");
  const skip = await runCheck(CHECKS.find((c) => c.id === "mdns"), { ...fakeLab({ subjects: BEFORE }), both: false });
  assert.equal(skip.status, "skip");
  assert.match(skip.detail, /needs vm1 and vm2 running; this checkout waits on link\/2 \(phase 5 commit 9\)/);
  const wait = await runCheck(CHECKS.find((c) => c.id === "gap"), fakeLab({ subjects: BEFORE }));
  assert.deepEqual(wait, { status: "wait", detail: "not in this build yet: waits on link/2 (phase 5 commit 9), holders (phase 5 commit 10)" });
  // No network yet: what the owner runs, never a failure.
  assert.deepEqual(await runCheck(restore, fakeLab({ network: false })), { status: "owner", detail: OWNER.join });
  for (const id of RELAY_CHECKS) {
    const ctx = fakeLab({ relays: false });
    const noRelay = await runCheck(CHECKS.find((c) => c.id === id), ctx);
    assert.equal(noRelay.status, "owner", id);
    assert.match(noRelay.detail, /add https:\/\/192\.168\.64\.2:3443 and https:\/\/192\.168\.64\.2:4443 .* as relays/, id);
    assert.deepEqual(ctx.lab.ca, ["vm1", "vm2"], `${id} has both VMs trust the lab's CA`);
    assert.equal(ctx.lab.off.size, 3, `${id} stops the relays it started`);
  }
  // A check that throws fails, with why.
  const boom = async () => {
    throw new Error("boom");
  };
  assert.deepEqual(await runCheck({ ...restore, run: boom }, fakeLab()), { status: "FAIL", detail: "boom" });
});

test("the relay checks fail when the relay sees plaintext, an unlisted relay is used, or the doctor doesn't name a relay that's down", async () => {
  const run = (id, ctx) => runCheck(CHECKS.find((c) => c.id === id), ctx);
  const clear = fakeLab();
  clear.capture = async (vm, verb) => (verb === "start" ? true : { bytes: 4096, found: true });
  assert.match((await run("relay-sealed", clear)).detail, /in the clear/);
  const noDump = fakeLab();
  noDump.capture = async () => false;
  assert.deepEqual(await run("relay-sealed", noDump), { status: "owner", detail: OWNER.tcpdump });
  const used = fakeLab();
  used.relayMetrics = async () => "relayserver_unique_client_keys_total 2\nrelayserver_bytes_recv_total 5\n";
  assert.match((await run("relay-unlisted", used)).detail, /vm1 contacted a relay its network doesn't list/);
  assert.deepEqual(used.lab.own, [], "vm2's own relay is taken out again");
  const quiet = fakeLab();
  const doctor = quiet.doctor;
  quiet.doctor = async () => JSON.stringify({ results: JSON.parse(await doctor()).results.map((r) => ({ ...r, status: "ok" })) });
  assert.match((await run("relay-down", quiet)).detail, /doesn't name https:\/\/192\.168\.64\.2:3443 as down/);
  assert.equal(metric("relayserver_accepts_total 7", "accepts"), 7);
  assert.equal(metric("nothing", "accepts"), null);
  assert.equal(relayRow("not json", "https://x"), null);
});

test("the networks check fails when anything of N1 reaches vm2", async () => {
  const ctx = fakeLab();
  const state = ctx.state;
  ctx.state = async (vm) => (vm === "vm2" ? { ...(await state(vm)), leaked: "net-1" } : state(vm));
  const r = await runCheck(CHECKS.find((c) => c.id === "networks"), ctx);
  assert.equal(r.status, "FAIL");
  assert.match(r.detail, /N1's id/);
});

test("the checks' firewall rules: UDP to a peer dropped, or a VM cut off its LAN but for one peer", () => {
  const udp = rules({ udpTo: ["192.168.64.3", "fe80::2"] });
  assert.match(udp, /^table inet xenonet_lab_check\ndelete table inet xenonet_lab_check\n/);
  assert.match(udp, /hook output priority -10; policy accept;\n {4}ip daddr \{ 192\.168\.64\.3 \} meta l4proto udp drop\n {4}ip6 daddr \{ fe80::2 \} meta l4proto udp drop/);
  assert.match(udp, /hook input .*\n {4}ip saddr \{ 192\.168\.64\.3 \} meta l4proto udp drop/);
  const cut = rules({ isolate: "lima0", allow: ["192.168.64.2"] });
  assert.match(cut, /oifname "lima0" ip daddr \{ 192\.168\.64\.2 \} accept\n {4}oifname "lima0" drop/);
  assert.match(cut, /iifname "lima0" ip saddr \{ 192\.168\.64\.2 \} accept\n {4}iifname "lima0" drop/);
  assert.match(rules({ isolate: "lima0" }), /oifname "lima0" drop/);
  assert.throws(() => rules({ udpTo: ["1.2.3.4; drop"] }), /not an address/);
  assert.throws(() => rules({ isolate: "lima0 drop" }), /not an interface/);
  assert.throws(() => rules({}), /no rules/);
});

test("a VM's networks and addresses, as its XenoNet and ip give them", () => {
  const before = { network: { id: "n", name: "lab" }, hosts: [{ id: "vm1" }, { id: "vm2" }] };
  assert.deepEqual(networksOf(before).map((n) => [n.id, n.hosts.length]), [["n", 2]]);
  assert.deepEqual(networksOf({ network: null, hosts: [] }), []);
  assert.deepEqual(networksOf({ networks: [{ id: "a", hosts: [] }, { id: "b", hosts: [] }] }).map((n) => n.id), ["a", "b"]);
  assert.equal(sharedNetwork(before, before).id, "n");
  assert.equal(sharedNetwork(before, { network: { id: "other" }, hosts: [{ id: "vm1" }, { id: "vm2" }] }), null);
  const ip = JSON.stringify([{ ifname: "lima0", addr_info: [{ family: "inet", local: "192.168.64.3" }, { family: "inet6", local: "fe80::5054:ff:fe12:3456" }] }]);
  assert.deepEqual(addrsOf(ip), ["192.168.64.3", "fe80::5054:ff:fe12:3456"]);
  assert.deepEqual(addrsOf("not json"), []);
});

test("guest.sh parses, and so does each Node script it runs in a VM", { skip: !bash && "Bash is required" }, () => {
  const text = fs.readFileSync(lab("guest.sh"), "utf8");
  assert.equal(runBash(["-n"], { input: guestScript("standin", { SESSION: "s", PROJECT: "atlas-api", CALLS: "[]" }, text) }).status, 0);
  const scripts = [...text.matchAll(/node - <<'EOF'\n([\s\S]*?)\nEOF\n/g)].map((m) => m[1]);
  assert.equal(scripts.length, 3, "standin, udpsend and udplisten");
  const dir = tmp();
  scripts.forEach((s, i) => {
    const file = path.join(dir, `guest-${i}.cjs`);
    fs.writeFileSync(file, s);
    const r = spawnSync(process.execPath, ["--check", file], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
  });
  // The steps lab.mjs runs are all there.
  for (const step of ["vm", "facts", "subjects", "lanaddrs", "netstate", "standin", "relay", "relayoff", "relayca", "trustca", "ownrelay", "relaymetrics", "udpsend", "udplisten", "answering"]) assert.match(text, new RegExp(`^${step}\\(\\) \\{`, "m"), step);
  assert.ok(!/authorized_keys\.new|hosts self|relay --port/.test(text), "nothing of the retired SSH hosts");
  assert.ok(!/iroh-relay"? --dev/.test(text), "the lab's relays are real ones, with TLS, never --dev");
});
