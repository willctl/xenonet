// The VM lab (scripts/lab): the files it writes on the Mac, the hash it
// gives each VM for sudo, the settings in its templates, and the made-up
// repos it puts in each VM. Nothing here starts a VM.
import { bash, runBash } from "./harness/bash.js";
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gitAvailable } from "../core/git.js";
import { BASES, VMS, LAB_INSTANCES, HOST_ID, labPaths, sshPath, sshConfigText, withHostKey, hostKeyOf, authorizedKeyLine, envFileText, guestScript, shq, sha512crypt, randomSalt, newPassword } from "../scripts/lab/config.mjs";
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
const KEY = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBq1sBYnXHeqn8D+pYeSGa1Nqv3Uv8ZQwRM4c3Xvm7eF";

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

test("the lab's files go in XenoNet Dev's state folder, or where XENONET_LAB_DIR says", () => {
  const mac = labPaths({ env: {}, home: H, platform: "darwin" });
  assert.equal(mac.dir, path.join(H, "Library", "Application Support", "XenoNet Dev", "lab"));
  assert.equal(mac.password("xenonet-vm1"), path.join(H, ".lima", "xenonet-vm1", "xenonet-lab-password"));
  assert.equal(labPaths({ env: { XDG_STATE_HOME: "/x" }, home: H, platform: "darwin" }).dir, path.join("/x", "xenonet-dev", "lab"));
  assert.equal(labPaths({ env: { XENONET_LAB_DIR: "/l", LIMA_HOME: "/m" }, home: H }).knownHosts, path.join(path.resolve("/l"), "known_hosts"));
  assert.equal(labPaths({ env: { LIMA_HOME: "/m" }, home: H }).instanceDir("xenonet-vm2"), path.join(path.resolve("/m"), "xenonet-vm2"));
});

test("the lab's ssh config: an alias per VM, the console key alone, host keys checked, no user name", () => {
  const paths = labPaths({ env: {}, home: H, platform: "darwin" });
  const text = sshConfigText({ paths, home: H });
  assert.ok(!text.includes(H), "the home folder is written as ~");
  assert.match(text, /^Host vm1\n {2}HostName 127\.0\.0\.1\n {2}Port 60851\n {2}HostKeyAlias xenonet-lab-vm1\n {2}User will\n/m);
  assert.match(text, /^Host vm2\n {2}HostName 127\.0\.0\.1\n {2}Port 60852\n/m);
  assert.ok(text.includes('IdentityFile "~/Library/Application Support/XenoNet Dev/lab/console_ed25519"'));
  assert.ok(text.includes('UserKnownHostsFile "~/Library/Application Support/XenoNet Dev/lab/known_hosts"'));
  for (const line of ["IdentitiesOnly yes", "IdentityAgent none", "StrictHostKeyChecking yes", "ForwardAgent no", "PasswordAuthentication no"]) {
    assert.equal(text.split("\n").filter((l) => l.trim() === line).length, Object.keys(VMS).length, line);
  }
  assert.equal(sshPath("/elsewhere/key", H), '"/elsewhere/key"');
  assert.throws(() => sshPath('/a"b', H), /ssh config/);
  assert.throws(() => sshConfigText({ vms: { VM1: { sshPort: 1 } }, paths, home: H }), /host id/);
});

test("known_hosts: a VM's line is replaced, the others kept, and only ed25519 keys go in", () => {
  const one = withHostKey("", "vm1", `${KEY} root@lima-xenonet-vm1`);
  assert.equal(one, `xenonet-lab-vm1 ${KEY}\n`, "the key's comment is left out");
  const two = withHostKey(one, "vm2", KEY.replace("Bq1", "Cq2"));
  const again = withHostKey(two, "vm1", KEY.replace("Bq1", "Dq3"));
  assert.equal(again.split("\n").filter(Boolean).length, 2);
  assert.equal(hostKeyOf(again, "vm1"), KEY.replace("Bq1", "Dq3"));
  assert.equal(hostKeyOf(again, "vm2"), KEY.replace("Bq1", "Cq2"));
  assert.equal(hostKeyOf(again, "vm3"), null);
  assert.throws(() => withHostKey("", "vm1", "ssh-rsa AAAAB3Nza"), /ed25519/);
  assert.throws(() => withHostKey("", "vm1", "ssh-ed25519 not;a;key"), /ed25519/);
});

test("the console key runs the relay and nothing else, on the port of the XenoNet under test", () => {
  assert.equal(authorizedKeyLine(`${KEY} anything`), `restrict,command="/home/will/.local/bin/xenonet relay --port 47841" ${KEY} xenonet-lab-console`);
  assert.match(authorizedKeyLine(KEY, 47840), /relay --port 47840" /);
  assert.throws(() => authorizedKeyLine(KEY, "47840; sh"), /port/);
  assert.throws(() => authorizedKeyLine(KEY, 70000), /port/);
  assert.throws(() => authorizedKeyLine('ssh-ed25519 AAAA" evil'), /ed25519/);
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

test("versions.json pins Node with its checksum, Claude Code and Codex", () => {
  const v = JSON.parse(fs.readFileSync(lab("versions.json"), "utf8"));
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

test("the repos script makes the same atlas-api history on two hosts, with a worktree and edits", { skip: (!bash || !gitAvailable()) && "Git and Bash are required" }, () => {
  const homes = [tmp(), tmp()];
  for (const [i, host] of [[0, "vm1"], [1, "vm2"]]) {
    const r = runBash( ["-s"], { input: reposScript(host), env: { PATH: process.env.PATH, HOME: homes[i] }, encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
  }
  const git = (home, rel, ...args) => spawnSync("git", ["-C", path.join(home, "Projects", rel), ...args], { encoding: "utf8", env: { PATH: process.env.PATH, HOME: home } }).stdout.trim();
  assert.equal(git(homes[0], "atlas-api", "rev-parse", "HEAD"), git(homes[1], "atlas-api", "rev-parse", "HEAD"));
  assert.equal(git(homes[0], "atlas-api", "status", "--short"), "M src/orders/routes.ts");
  assert.equal(git(homes[0], "atlas-api-refunds", "rev-parse", "--abbrev-ref", "HEAD"), "feat/partial-refunds");
  assert.equal(git(homes[0], "atlas-api-refunds", "status", "--short"), "?? src/orders/refunds.test.ts");
  assert.equal(git(homes[0], "Café Zones", "log", "-1", "--format=%an <%ae>"), "Nova <nova@example.com>");
  assert.equal(git(homes[0], "labs/geofence", "config", "user.name"), "Orbit");
  assert.ok(fs.existsSync(path.join(homes[1], "Projects", "atlas-kit", "pnpm-workspace.yaml")));
  assert.ok(!fs.existsSync(path.join(homes[1], "Projects", "atlas-web")));
  // A second run ends the same.
  const before = git(homes[0], "atlas-api", "rev-parse", "HEAD");
  assert.equal(runBash( ["-s"], { input: reposScript("vm1"), env: { PATH: process.env.PATH, HOME: homes[0] } }).status, 0);
  assert.equal(git(homes[0], "atlas-api", "rev-parse", "HEAD"), before);
});
