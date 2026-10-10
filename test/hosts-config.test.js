// hosts.json and XenoNet's own ssh config (core/hosts/config.js), in temp
// folders. Every host here is made up.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { otherReaders } from "../core/private-file.js";
import os from "node:os";
import path from "node:path";
import { HostsConfig, hostEntry, parseTarget, sshConfigText } from "../core/hosts/config.js";
import { check } from "../core/schema/index.js";

const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-hosts-config-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

const LAB = { id: "vm1", target: "lab@127.0.0.1", sshPort: 60851, identity: "~/Library/Application Support/XenoNet Dev/lab/console_ed25519", knownHosts: "~/Library/Application Support/XenoNet Dev/lab/known_hosts", hostKeyAlias: "xenonet-lab-vm1", command: "/home/lab/.local/bin/xenonet" };

test("this XenoNet's own key is made once, the first time anything asks, and kept", () => {
  const dir = tmp();
  const hosts = new HostsConfig({ stateDir: dir });
  assert.equal(fs.existsSync(path.join(dir, "hosts.json")), false, "nothing written before it's needed");
  const self = hosts.self();
  assert.match(self.key, /^[0-9a-f]{32}$/);
  assert.equal(self.id, null, "never named after the machine");
  assert.deepEqual(hosts.self(), self);
  assert.deepEqual(new HostsConfig({ stateDir: dir }).self(), self, "read back from hosts.json");
  assert.equal(hosts.setSelf("studio").id, "studio");
  assert.throws(() => hosts.setSelf("Novas-MacBook-Pro"), /name you choose/);
  assert.equal(new HostsConfig({ stateDir: dir }).self().key, self.key, "renaming keeps the key");
  assert.equal(hosts.setSelf(null).id, null);
  const file = JSON.parse(fs.readFileSync(path.join(dir, "hosts.json"), "utf8"));
  assert.ok(check("host/list/v1", file, { strict: true }).ok);
});

test("an ssh host goes into hosts.json and into XenoNet's own ssh config, and comes out again", () => {
  const dir = tmp();
  const hosts = new HostsConfig({ stateDir: dir });
  const vm1 = hosts.add(LAB);
  assert.deepEqual(vm1.ssh, { hostname: "127.0.0.1", user: "lab", port: 60851, identity: LAB.identity, knownHosts: LAB.knownHosts, hostKeyAlias: "xenonet-lab-vm1" });
  assert.deepEqual([vm1.transport, vm1.port, vm1.command, vm1.since, vm1.detail, vm1.key], ["ssh", 47840, LAB.command, "7d", false, null]);
  const config = fs.readFileSync(path.join(dir, "ssh_config"), "utf8");
  assert.match(config, /^# XenoNet's own ssh config/);
  assert.match(config, /\nHost vm1\n  HostName 127\.0\.0\.1\n  User lab\n  Port 60851\n  IdentityFile "~\/Library\/Application Support\/XenoNet Dev\/lab\/console_ed25519"\n  IdentitiesOnly yes\n  UserKnownHostsFile "~\/Library\/Application Support\/XenoNet Dev\/lab\/known_hosts"\n  HostKeyAlias xenonet-lab-vm1\n$/);
  assert.deepEqual(otherReaders(path.join(dir, "ssh_config")), [], "ssh refuses a config others can write");

  // Another XenoNet on this machine has no ssh block.
  hosts.add({ id: "xenonet", local: true, port: 47840 });
  assert.doesNotMatch(fs.readFileSync(path.join(dir, "ssh_config"), "utf8"), /Host xenonet/);
  // A bare id is its own target.
  assert.equal(hosts.add({ id: "mac1" }).ssh.hostname, "mac1");
  assert.deepEqual(hosts.list().map((h) => h.id), ["vm1", "xenonet", "mac1"]);

  assert.throws(() => hosts.add(LAB), (err) => err.status === 409 && /already a host called vm1/.test(err.message));
  hosts.learn("vm1", { key: "9a2b0000000000000000000000000002", name: "lab-one" });
  assert.deepEqual([hosts.get("vm1").key, hosts.get("vm1").name], ["9a2b0000000000000000000000000002", "lab-one"]);
  assert.equal(hosts.add({ ...LAB, target: "lab@127.0.0.2" }, { replace: true }).key, null, "adding again forgets the key it learned");

  assert.equal(hosts.remove("vm1").id, "vm1");
  assert.equal(hosts.remove("vm1"), null);
  assert.doesNotMatch(fs.readFileSync(path.join(dir, "ssh_config"), "utf8"), /Host vm1/);
  const file = JSON.parse(fs.readFileSync(path.join(dir, "hosts.json"), "utf8"));
  assert.ok(check("host/list/v1", file, { strict: true }).ok);
  assert.deepEqual(file.hosts.map((h) => h.id), ["xenonet", "mac1"]);
});

test("nothing that could become an ssh option, a shell command or a second line gets in", () => {
  const refused = [
    [{ id: "VM1" }, /name you choose/],
    [{ id: "vm1", target: "-oProxyCommand=open -a Calculator" }, /ssh target|host name/],
    [{ id: "vm1", target: "lab@-oProxyCommand=x" }, /host name/],
    [{ id: "vm1", target: "la b@host" }, /ssh target/],
    [{ id: "vm1", target: "a@b@host" }, /ssh target/],
    [{ id: "vm1", command: "xenonet; rm -rf ~" }, /xenonet command/],
    [{ id: "vm1", command: "$(id)" }, /xenonet command/],
    [{ id: "vm1", command: "relative/xenonet" }, /xenonet command/],
    [{ id: "vm1", identity: "/tmp/key\nProxyCommand evil" }, /key file/],
    [{ id: "vm1", identity: "key" }, /key file/],
    [{ id: "vm1", knownHosts: '/tmp/"known' }, /known_hosts/],
    [{ id: "vm1", identity: "/tmp/%h" }, /key file/],
    [{ id: "vm1", hostKeyAlias: "a b" }, /host key/],
    [{ id: "vm1", port: 0 }, /port/],
    [{ id: "vm1", port: "47840; id" }, /port/],
    [{ id: "vm1", sshPort: 70000 }, /ssh's port/],
    [{ id: "vm1", since: "forever" }, /how far back/],
    [{ id: "vm1", transport: "telnet" }, /over ssh, or locally/],
  ];
  for (const [input, message] of refused) assert.throws(() => hostEntry(input), (err) => err.status === 400 && message.test(err.message), JSON.stringify(input));
  assert.deepEqual(parseTarget("vm1.local"), { user: null, hostname: "vm1.local" });
  // Even an entry that got past the checks some other way is left out of
  // the config file.
  assert.equal(sshConfigText([{ ...hostEntry({ id: "vm1" }), ssh: { hostname: "x\nProxyCommand evil", user: null, port: null, identity: null, knownHosts: null, hostKeyAlias: null } }]).includes("ProxyCommand"), false);
});

test("a hosts.json someone edited by hand: entries XenoNet can't use are left out, and a file it can't read is moved aside", () => {
  const dir = tmp();
  const good = hostEntry({ id: "vm1" });
  fs.writeFileSync(path.join(dir, "hosts.json"), JSON.stringify({ schema: "host/list/v1", self: { id: "Bad Name", key: "7c1e0000000000000000000000000001", createdAt: 1 }, hosts: [good, { ...good }, { ...good, id: "vm2", command: "xenonet && evil" }, { id: "vm3" }] }));
  const hosts = new HostsConfig({ stateDir: dir });
  assert.deepEqual(hosts.list().map((h) => h.id), ["vm1"]);
  assert.deepEqual(hosts.dropped, ["vm1", "vm2", "vm3"], "the duplicate, the bad command and the half entry");
  assert.deepEqual(hosts.self(), { id: null, key: "7c1e0000000000000000000000000001", createdAt: 1 }, "the key stays; a name that isn't one goes");

  const broken = tmp();
  fs.writeFileSync(path.join(broken, "hosts.json"), "{ not json");
  const fresh = new HostsConfig({ stateDir: broken });
  assert.deepEqual(fresh.list(), []);
  assert.ok(fs.readdirSync(broken).some((f) => /^hosts\.broken-.+\.json$/.test(f)), "moved aside, never written over");

  const newer = tmp();
  fs.writeFileSync(path.join(newer, "hosts.json"), JSON.stringify({ schema: "host/list/v2", self: null, hosts: [] }));
  const later = new HostsConfig({ stateDir: newer });
  assert.throws(() => later.add({ id: "vm1" }), (err) => err.status === 409 && /newer XenoNet/.test(err.message));
  assert.equal(JSON.parse(fs.readFileSync(path.join(newer, "hosts.json"), "utf8")).schema, "host/list/v2", "a newer XenoNet's file stays as it is");
});
