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
import { XCHECKS, XLANDS, XOWNER, NAMES, OLD_TAG, REHEARSAL, SPACE, folderPath, rehearsalVerdict, runXCheck, xenolithOps, xmissing } from "../scripts/lab/xenolith.mjs";

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

// ---------- Xenolith in the lab ----------

// The subjects of the commits on main that land Xenolith's pieces, and the one the grant's
// minting will need (not built yet).
const XLANDED = ["feat(xenolith): upgrade gate", "feat(xenolith): sync/2 on Xenolith's own endpoint", "feat(xenolith): join/1 replaces pairing", "feat(xenolith): fetch the curator's feed from any member holding it", "feat(xenolith,service): shared mode over the forward"];
const XGRANT = "feat(service): mint Xenolith's grant in the window";

test("a Xenolith check waits on the commits it needs, found by their subjects, the grant's minting among them", () => {
  assert.deepEqual(xmissing(Object.keys(XLANDS), XLANDED), ["Xenolith's grant"]);
  assert.deepEqual(xmissing(Object.keys(XLANDS), [...XLANDED, XGRANT]), []);
  assert.deepEqual(xmissing(Object.keys(XLANDS), ["docs(xenolith): sync/2 and join/1, the upgrade, holders, shared mode", "feat(net): link/2"]), Object.keys(XLANDS), "only a feat commit lands a piece");
  assert.deepEqual(xmissing(["shared mode"], ["feat(service): shared mode for XenoNet"]), ["shared mode"], "shared mode is Xenolith's");
});

test("the Xenolith checks: each names its VMs, what it needs, the owner's step, an agent's stand-in and what it changes", () => {
  const ids = XCHECKS.map((c) => c.id);
  assert.deepEqual(ids, ["xenolith-join", "xenolith-notes", "xenolith-offline", "xenolith-holder", "xenolith-shared", "xenolith-rehearsal"]);
  assert.deepEqual(ids.filter((id) => CHECKS.some((c) => c.id === id)), [], "--only can name either kind");
  for (const c of XCHECKS) {
    assert.ok(c.what && c.changes && c.owner && c.standIn && typeof c.run === "function", c.id);
    for (const n of c.needs) assert.ok(XLANDS[n], `${c.id} needs ${n}`);
    for (const vm of c.vms) assert.ok(NAMES[vm], `${c.id} runs in ${vm}`);
  }
  const by = Object.fromEntries(XCHECKS.map((c) => [c.id, c]));
  assert.deepEqual(by["xenolith-holder"].vms, ["vm1", "vm2", "vm3"]);
  assert.deepEqual(by["xenolith-rehearsal"].vms, ["vm1"], "1.x serves on 127.0.0.1 only, so both 1.x Xenoliths are in vm1");
  assert.ok(by["xenolith-shared"].needs.includes("Xenolith's grant"));
  assert.match(XOWNER.old, /git fetch origin tag xenolith-v1\.10\.0/);
});

// A made-up lab of Xenoliths in vm1, vm2 and vm3 that behaves as sync/2 does: a member running
// beside a running curator holds the curator's pages and sends its notes, stamped as its key; with
// the curator stopped, a member takes the pages another running member holds.
function fakeXenolith({ subjects = [...XLANDED, XGRANT], vm3 = true, running = {}, network = true, attached = true, shared = true, tag = true, ...o } = {}) {
  let now = 0;
  const vms = ["vm1", "vm2", ...(vm3 ? ["vm3"] : [])];
  const at = Object.fromEntries(vms.map((vm) => [vm, { made: false, up: false, files: new Map(), held: new Map(), outbox: [], member: false, level: null, wasDown: false }]));
  const lab = { at, invites: new Map(), founded: false, notes: 0, codeInArgs: false, resets: 0 };
  const dir = `spaces/${SPACE.replace(":", "/")}`;
  const members = () => vms.filter((v) => v !== "vm1" && at[v].member);
  const sync = () => {
    const cur = at.vm1;
    if (cur.up && lab.founded) {
      const pages = [...cur.files].filter(([k]) => k.startsWith(`${dir}/pages/`));
      for (const m of members()) {
        if (!at[m].up || o.noSync || (o.noCatchUp && at[m].wasDown)) continue;
        at[m].held = new Map(pages);
        for (const [k, v] of pages) at[m].files.set(k, v);
        for (const t of at[m].outbox.splice(0)) cur.files.set(`${dir}/inbox/n-${++lab.notes}.md`, `---\ntitle: ${t}\ntransport: sync\nby: "key:k-${m}"\nsource: ${o.stamp ?? "agent"}\n---\n${t}\n`);
      }
    } else if (!o.withhold) {
      for (const m of members())
        for (const h of members())
          if (h !== m && at[m].up && at[h].up) for (const [k, v] of at[h].held) if (at[m].files.get(k) !== v) (at[m].files.set(k, v), at[m].held.set(k, v));
    }
  };
  const status = (vm) => {
    const a = at[vm];
    const curated = [{ id: "spc_lab", address: SPACE, role: "curator", curator: "k-vm1", members: members().map((m) => ({ key: `k-${m}`, endpoint: o.wrongEndpoint && m === "vm2" ? "e-other" : `e-${m}`, level: at[m].level, connected: at[m].up })), networks: shared ? [{ network: "net-2", level: "read" }] : [] }];
    const followed = a.member || (vm !== "vm1" && attached && shared) ? [{ id: "spc_lab", address: SPACE, role: "member", curator: "k-vm1" }] : [];
    return { key: `k-${vm}`, endpointId: `e-${vm}`, spaces: vm === "vm1" ? (lab.founded ? curated : []) : followed, xenonet: { mode: attached ? "attached" : "waiting", peers: vm === "vm1" ? [] : [{ key: "k-vm1", machine: "vm1", connected: true }] } };
  };
  const cli = async (vm, name, args, input = "") => {
    assert.equal(name, NAMES[vm]);
    const a = at[vm];
    if (!a.made || !a.up) return { code: 1, out: "", err: "xenolith sync: The Xenolith service on port 41234 is not answering (refused). Start it, then try again." };
    sync();
    if ([...lab.invites.values()].some((i) => args.includes(i.code))) lab.codeInArgs = true;
    if (args[0] === "sync") return { code: 0, out: JSON.stringify({ sync: status(vm) }), err: "" };
    if (args[0] === "invite") {
      if (o.inviteRefused) return { code: 1, out: "", err: "xenolith sync: No running Xenolith service has left an owner key you can read (~/x). Start the service, then try again." };
      const invite = `xlj1_${lab.invites.size}`;
      const code = `C0DE-${lab.invites.size}-XYZW`;
      lab.invites.set(invite, { code, level: args[3], used: false });
      return { code: 0, out: JSON.stringify({ invite, code, level: args[3], offer: { spaces: [{ space: "spc_lab", address: SPACE }] } }), err: "" };
    }
    if (args[0] === "join") {
      const inv = lab.invites.get(args[1]);
      if (o.joinRefused || !inv || inv.used || input.trim() !== inv.code) return { code: 1, out: "", err: "xenolith sync: the code doesn't match the invite: 4 tries left (wrong-code)" };
      if (args[3] !== "192.168.64.2:47851") return { code: 1, out: "", err: `xenolith sync: couldn't reach ${args[3]}` };
      Object.assign(inv, { used: true });
      Object.assign(a, { member: true, level: inv.level });
      sync();
      return { code: 0, out: JSON.stringify({ followed: [{ space: "spc_lab", address: SPACE }], level: inv.level }), err: "" };
    }
    if (args[0] === "note") {
      if (!a.member) return { code: 1, out: "", err: `xenolith note: No space "${SPACE}" here.` };
      a.outbox.push(args[1]);
      sync();
      return { code: 0, out: "{}", err: "" };
    }
    return { code: 2, out: "", err: "usage" };
  };
  const x = {
    hasRef: async (ref) => tag && ref === OLD_TAG,
    build: async (vm) => (o.buildFails === vm ? { ok: false, why: "error: linking with `cc` failed" } : { ok: true }),
    reset: async (vm, name, { space, page } = {}) => {
      lab.resets++;
      Object.assign(at[vm], { made: true, up: true, files: new Map(), held: new Map(), outbox: [], member: false, level: null, wasDown: false });
      if (vm === "vm1") {
        lab.founded = Boolean(space);
        if (space) at.vm1.files.set(`${dir}/pages/lab.md`, page);
      }
    },
    start: async (vm) => {
      if (at[vm].made) at[vm].up = true;
      sync();
    },
    stop: async (vm) => Object.assign(at[vm], { up: false, wasDown: true }),
    cli,
    read: async (vm, name, rel) => (sync(), at[vm].files.get(folderPath(rel)) ?? null),
    write: async (vm, name, rel, text) => (at[vm].files.set(folderPath(rel), text), sync()),
    find: async (vm, name, rel, needle) => (sync(), [...at[vm].files].filter(([k, v]) => k.startsWith(`${rel}/`) && v.includes(needle)).map(([, v]) => v)),
    old: async () => (o.oldFails ? { ok: false, why: "error[E0599]: no method" } : { ok: true }),
    rehearse: async () => o.rehearsal ?? REHEARSAL.map((s) => `stage=${s} ok ${s} went as planned`).join("\n"),
  };
  return {
    lab,
    now: () => now,
    sleep: async (ms) => {
      now += ms;
    },
    has: (vm) => vms.includes(vm) && (running[vm] ?? true),
    subjects: async () => subjects,
    lanAddrs: async (vm) => (vm === "vm1" ? ["fe80::1", "192.168.64.2"] : ["192.168.64.3"]),
    state: async () => (network ? { networks: [{ id: "net-2", hosts: [{ id: "vm1" }, { id: "vm2" }] }] } : { network: null, hosts: [] }),
    x,
  };
}

const xcheck = (id) => XCHECKS.find((c) => c.id === id);

test("every Xenolith check passes on a lab that behaves; the code never goes into an argument, and what a check stops it starts again", async () => {
  for (const c of XCHECKS) {
    const ctx = fakeXenolith();
    const r = await runXCheck(c, ctx);
    assert.equal(r.status, "ok", `${c.id}: ${r.detail}`);
    assert.equal(ctx.lab.codeInArgs, false, `${c.id}: the join's code went on stdin`);
    for (const vm of ["vm1", "vm2", "vm3"]) if (ctx.lab.at[vm].made) assert.equal(ctx.lab.at[vm].up, true, `${c.id} leaves ${vm}'s Xenolith running`);
  }
  // In one run, the later checks use the join the first made.
  const ctx = fakeXenolith();
  for (const id of ["xenolith-join", "xenolith-notes", "xenolith-offline"]) assert.equal((await runXCheck(xcheck(id), ctx)).status, "ok", id);
  assert.equal(ctx.lab.invites.size, 1, "one invite for the run");
  assert.equal(ctx.lab.resets, 2, "vm1 and vm2 made once");
});

test("a Xenolith check skips without its VMs, waits on missing code, and says what the owner runs", async () => {
  const skip = await runXCheck(xcheck("xenolith-join"), fakeXenolith({ running: { vm2: false }, subjects: XLANDED.slice(0, 1) }));
  assert.equal(skip.status, "skip");
  assert.match(skip.detail, /^needs vm1 and vm2 running; this checkout waits on sync\/2 \(phase 6 commit 13\), join\/1 \(phase 6 commit 14\)$/);
  assert.deepEqual(await runXCheck(xcheck("xenolith-holder"), fakeXenolith({ vm3: false, subjects: XLANDED })), { status: "skip", detail: "the lab has no vm3 running" });
  // The rehearsal needs vm1 alone.
  assert.equal((await runXCheck(xcheck("xenolith-rehearsal"), fakeXenolith({ running: { vm2: false } }))).status, "ok");
  // Shared mode waits on the grant's minting, which main hasn't built.
  assert.deepEqual(await runXCheck(xcheck("xenolith-shared"), fakeXenolith({ subjects: XLANDED })), { status: "wait", detail: "not in this build yet: waits on Xenolith's grant (not built yet: XenoNet minting Xenolith's grant)" });
  const shared = (o) => runXCheck(xcheck("xenolith-shared"), fakeXenolith(o));
  assert.deepEqual(await shared({ network: false }), { status: "owner", detail: OWNER.join });
  const detached = await shared({ attached: false });
  assert.equal(detached.status, "owner");
  assert.match(detached.detail, /^give the Xenolith in vm1 and in vm2 its grant .*\(not attached: vm1, vm2\)$/);
  assert.deepEqual(await shared({ shared: false }), { status: "owner", detail: XOWNER.share });
  // The invite needs the owner key the service left: without it, the owner's own steps.
  assert.deepEqual(await runXCheck(xcheck("xenolith-join"), fakeXenolith({ inviteRefused: true })), { status: "owner", detail: XOWNER.join });
  assert.match(XOWNER.join, /invite project:atlas-api --level write; in vm2, .* join <the invite> --at <vm1's lima0 address>:47851, and type the code/);
  // No 1.10.0 here: how to get it.
  assert.deepEqual(await runXCheck(xcheck("xenolith-rehearsal"), fakeXenolith({ tag: false })), { status: "owner", detail: XOWNER.old });
  // A check that throws fails, with why.
  const ctx = fakeXenolith();
  ctx.x.reset = async () => {
    throw new Error("vm1: the lab Xenolith curator didn't start (Xenolith curator didn't answer in 30 s)");
  };
  assert.deepEqual(await runXCheck(xcheck("xenolith-join"), ctx), { status: "FAIL", detail: "vm1: the lab Xenolith curator didn't start (Xenolith curator didn't answer in 30 s)" });
});

test("the Xenolith checks fail when a build, a join, sync, a note's stamp, a catch-up or a holder goes wrong", async () => {
  const run = async (id, o) => runXCheck(xcheck(id), fakeXenolith(o));
  assert.deepEqual(await run("xenolith-join", { buildFails: "vm2" }), { status: "FAIL", detail: "vm2: Xenolith didn't build (error: linking with `cc` failed)" });
  assert.deepEqual(await run("xenolith-join", { joinRefused: true }), { status: "FAIL", detail: "vm2's join was refused: xenolith sync: the code doesn't match the invite: 4 tries left (wrong-code)" });
  assert.deepEqual(await run("xenolith-join", { noSync: true }), { status: "FAIL", detail: "vm1's page didn't reach vm2 in 60 s" });
  assert.match((await run("xenolith-join", { wrongEndpoint: true })).detail, /names e-other at write, not vm2's endpoint at write/);
  assert.deepEqual(await run("xenolith-notes", { noSync: true }), { status: "FAIL", detail: "vm2's note didn't reach vm1's inbox in 30 s" });
  assert.deepEqual(await run("xenolith-notes", { stamp: "user" }), { status: "FAIL", detail: "a peer's note speaks as the person in vm1's inbox" });
  const behind = fakeXenolith({ noCatchUp: true });
  assert.deepEqual(await runXCheck(xcheck("xenolith-offline"), behind), { status: "FAIL", detail: "vm2 didn't catch up on vm1's page in 60 s" });
  assert.ok(behind.lab.at.vm1.up && behind.lab.at.vm2.up, "both started again");
  const withheld = fakeXenolith({ withhold: true });
  assert.deepEqual(await runXCheck(xcheck("xenolith-holder"), withheld), { status: "FAIL", detail: "vm2 didn't get the page from vm3 in 60 s with vm1 stopped" });
  assert.ok(withheld.lab.at.vm1.up && withheld.lab.at.vm2.up, "both started again");
  assert.deepEqual(await run("xenolith-rehearsal", { oldFails: true }), { status: "FAIL", detail: "vm1: Xenolith 1.10.0 didn't build (error[E0599]: no method)" });
  assert.deepEqual(await run("xenolith-rehearsal", { rehearsal: "stage=built ok 1.x 1.10.0\nstage=1x-synced ok paired\nstage=queued ok waiting\nstage=upgraded FAIL neo's queued note isn't kept with its moved-aside copy" }), { status: "FAIL", detail: "upgraded: neo's queued note isn't kept with its moved-aside copy" });
});

test("the rehearsal's verdict: every stage ok, the first FAIL, or where it stopped", () => {
  assert.deepEqual(REHEARSAL, ["built", "1x-synced", "queued", "upgraded", "rolledback", "1x-again"]);
  const all = REHEARSAL.map((s) => `stage=${s} ok ${s} done`).join("\n");
  assert.deepEqual(rehearsalVerdict(`noise\n${all}\n`), { status: "ok", detail: REHEARSAL.map((s) => `${s} done`).join("; ") });
  assert.deepEqual(rehearsalVerdict("stage=built ok b\nstage=1x-synced FAIL inix's page didn't reach neo"), { status: "FAIL", detail: "1x-synced: inix's page didn't reach neo" });
  assert.deepEqual(rehearsalVerdict("stage=built ok b\nstage=1x-synced ok s"), { status: "FAIL", detail: "the rehearsal stopped before queued, after 1x-synced" });
  assert.deepEqual(rehearsalVerdict(""), { status: "FAIL", detail: "the rehearsal stopped before built" });
});

test("Xenolith's steps in a VM: the code on stdin, paths inside the folder, 1.10.0 only when this checkout has it", async () => {
  const calls = [];
  const answers = { xread: { status: 3, stdout: "" }, xbuild: { status: 101, stdout: "", stderr: "Compiling xenolith-core\nerror: could not compile `xenolith-net`" } };
  const guest = async (vm, step, vars, opts) => (calls.push({ vm, step, vars, opts }), answers[step] ?? { status: 0, stdout: step === "xfind" ? '["a note"]' : "out", stderr: "" });
  let pushed = 0;
  const x = xenolithOps({ guest, push: async (vm, ref, branch) => (pushed++, ref === OLD_TAG && branch === "xenolith-1x" ? true : null), hasRef: async () => true });
  const r = await x.cli("vm2", "member", ["join", "xlj1_abc", "--json"], "C0DE\n");
  assert.deepEqual(r, { code: 0, out: "out", err: "" });
  const c = calls.at(-1);
  assert.deepEqual([c.step, c.vars.ARGS, c.vars.INPUT], ["xcli", '["join","xlj1_abc","--json"]', "C0DE\n"]);
  assert.equal(await x.read("vm1", "curator", "spaces/project/atlas-api/pages/lab.md"), null, "a file that isn't there");
  assert.deepEqual(await x.find("vm1", "curator", "spaces/project/atlas-api/inbox", "n"), ["a note"]);
  await x.write("vm1", "curator", "spaces/project/atlas-api/pages/x.md", "é");
  assert.equal(Buffer.from(calls.at(-1).vars.TEXT, "base64").toString("utf8"), "é");
  await x.reset("vm1", "curator", { space: SPACE, page: "p" });
  assert.deepEqual([calls.at(-1).vars.SPACE, calls.at(-1).vars.PAGE, calls.at(-1).vars.BIND], [SPACE, Buffer.from("p").toString("base64"), ""]);
  assert.deepEqual(await x.build("vm1"), { ok: false, why: "error: could not compile `xenolith-net`" });
  assert.deepEqual(await x.old("vm1"), { ok: true });
  assert.equal(pushed, 1);
  for (const bad of ["../home/runtime/owner.key", "/etc/passwd", "spaces//x", "spaces/$(id)"]) assert.throws(() => folderPath(bad), /not a path in the folder/, bad);
  await assert.rejects(x.read("vm1", "curator", "../x"), /not a path in the folder/);
  const none = xenolithOps({ guest, push: async () => null, hasRef: async () => false });
  const before = calls.length;
  assert.deepEqual(await none.old("vm1"), { ok: false, why: `no ${OLD_TAG} in this checkout` });
  assert.equal(calls.length, before, "no build without the tag");
});

test("xenolith.sh parses, so does each Node script it runs, and it has every step xenolith.mjs runs", { skip: !bash && "Bash is required" }, () => {
  const text = fs.readFileSync(lab("xenolith.sh"), "utf8");
  assert.equal(runBash(["-n"], { input: guestScript("xcli", { NAME: "member", ARGS: "[]", INPUT: "" }, text) }).status, 0);
  const scripts = [...text.matchAll(/node - [^\n]*<<'EOF'\n([\s\S]*?)\nEOF\n/g)].map((m) => m[1]);
  assert.equal(scripts.length, 1, "xfind");
  const dir = tmp();
  scripts.forEach((s, i) => {
    const file = path.join(dir, `xenolith-${i}.cjs`);
    fs.writeFileSync(file, s);
    const r = spawnSync(process.execPath, ["--check", file], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
  });
  const mjs = fs.readFileSync(lab("xenolith.mjs"), "utf8");
  const steps = [...mjs.matchAll(/(?:guest|must)\(vm, "(x[a-z]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...new Set(steps)].sort(), ["xbuild", "xcli", "xfind", "xold", "xread", "xrehearse", "xreset", "xstart", "xstop", "xwrite"]);
  for (const step of steps) assert.match(text, new RegExp(`^${step}\\(\\) \\{`, "m"), step);
  // A join's code never goes into an argument, and nothing here installs Xenolith or uses port 47850.
  assert.doesNotMatch(text, /pair join[^\n]*\$code|join[^\n]*"\$INPUT"/);
  assert.doesNotMatch(text, /xenolith"? install\b|47850/);
});
