// xenonet hosts and xenonet relay's arguments, against a stand-in XenoNet: what
// each says, what it asks XenoNet to do, and its exit codes. Every host here
// is made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { command } from "./harness/xenonet-bin.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const KEY = "7c1e0000000000000000000000000001";

// `routes` maps "GET /api/x" to (url, body) => [status, data].
async function standIn(routes) {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const url = new URL(req.url, "http://x");
      const parsed = body ? JSON.parse(body) : null;
      seen.push({ method: req.method, path: url.pathname, search: url.search, body: parsed });
      const route = routes[`${req.method} ${url.pathname}`];
      const [status, data] = route ? route(url, parsed) : [404, { error: "not found" }];
      res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(data));
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, close: () => server.close() };
}

function xenonet(args) {
  return new Promise((resolve) => {
    const proc = spawn(...command(args), { cwd: root, env: { ...process.env, NO_COLOR: "1", XENONET_PORT: "", HOME: "/Users/nova", USERPROFILE: "/Users/nova" }, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.stderr.on("data", (d) => (stderr += d));
    proc.stdin.end();
    proc.on("exit", (code) => resolve({ code, stdout, stderr }));
  });
}

const health = { ok: true, name: "XenoNet Dev", version: "0.4.0", demo: false, sources: [] };
const now = Date.now();
const vm1 = { id: "vm1", state: "live", since: now - 60e3, lastHeard: now, offsetMs: 300, version: "0.4.0", platform: "linux", demo: true, calls: null, error: null, nextTryAt: null, cached: false, transport: "ssh", target: "lab@127.0.0.1:60851", port: 47841 };
const mac1 = { ...vm1, id: "mac1", state: "error", target: "mac1.local", version: null, platform: null, demo: null, lastHeard: null, nextTryAt: now + 60e3, error: { code: "host-key", message: "ssh doesn't know mac1's host key, or it changed", fix: "Check it and accept it once in a terminal: ssh -F ~/'Library/Application Support/XenoNet Dev/ssh_config' mac1" } };

test("xenonet hosts: hosts off, the list with what's wrong and how to fix it, and this XenoNet's own name", async () => {
  let on = false;
  let self = { id: null, key: KEY, createdAt: 1, home: "/Users/nova" };
  const s = await standIn({
    "GET /api/health": () => [200, health],
    "GET /api/hosts": () => (on ? [200, { self, sshConfig: "/Users/nova/Library/Application Support/XenoNet Dev/ssh_config", hosts: [vm1, { ...mac1, nextTryAt: Date.now() + 60e3 }] }] : [404, { error: "hosts are off" }]),
    "GET /api/hosts/self": () => [200, self],
    "POST /api/hosts/self": (_url, body) => ((self = { ...self, id: body.id }), [200, self]),
  });
  const port = ["--port", String(s.port)];
  try {
    const offList = await xenonet(["hosts", ...port]);
    assert.equal(offList.code, 0);
    assert.match(offList.stdout, /^Hosts are off in XenoNet Dev: start it with XENONET_HOSTS=1/);
    assert.match(offList.stdout, /As a host, it hasn't named itself: xenonet hosts self <name>\./);
    assert.deepEqual(JSON.parse((await xenonet(["hosts", "--json", ...port])).stdout).on, false);

    on = true;
    // Lines wrap at the terminal's width: read it as one.
    const list = (await xenonet(["hosts", ...port])).stdout.replace(/\s+/g, " ");
    assert.match(list, /^● this XenoNet · XenoNet Dev, here · key 7c1e0000…/);
    assert.match(list, /vm1 lab@127\.0\.0\.1:60851 ● live XenoNet 0\.4\.0 on linux · made-up data/);
    assert.match(list, /mac1 mac1\.local ✗ error ssh doesn't know mac1's host key, or it changed · never reached, trying again in \d+ s/);
    assert.match(list, /↳ Check it and accept it once in a terminal: ssh -F ~\/'Library\/Application Support\/XenoNet Dev\/ssh_config' mac1/);
    assert.match(list, /XenoNet reaches them with its own ssh config, ~\/Library\/Application Support\/XenoNet Dev\/ssh_config\./);

    assert.match((await xenonet(["hosts", "self", ...port])).stdout, /XenoNet Dev hasn't named itself as a host \(key 7c1e0000…\)/);
    assert.match((await xenonet(["hosts", "self", "studio", ...port])).stdout, /✓ XenoNet Dev calls itself studio as a host now/);
    assert.match((await xenonet(["hosts", "self", ...port])).stdout, /XenoNet Dev calls itself studio as a host/);
    assert.match((await xenonet(["hosts", "self", "--clear", ...port])).stdout, /has no host name now/);
    assert.deepEqual(s.seen.filter((x) => x.method === "POST").map((x) => x.body), [{ id: "studio" }, { id: null }]);
  } finally {
    s.close();
  }
});

test("xenonet hosts add, remove, test and reconnect: what each asks XenoNet for, and says", async () => {
  let tested = { id: "vm1", ok: true, ms: 1234, state: "live", error: null, hello: { id: "lab", version: "0.4.0", platform: "linux", demo: true, protocol: 1, sinceMs: 7 * 86400e3, detail: false }, offsetMs: 300, snapshot: { nodes: 32, sessions: 14, live: 9 } };
  const s = await standIn({
    "GET /api/health": () => [200, health],
    "GET /api/hosts": () => [200, { self: { id: "studio", key: KEY }, sshConfig: "/Users/nova/Library/Application Support/XenoNet Dev/ssh_config", hosts: [vm1] }],
    "POST /api/hosts": (_url, body) => [200, { ...vm1, id: body.id, target: body.transport === "local" ? `this machine, port ${body.port}` : vm1.target, transport: body.transport ?? "ssh" }],
    "DELETE /api/hosts/vm1": (url) => [200, { id: "vm1", removed: true, forgot: url.searchParams.get("forget") === "1" ? 4 : 0 }],
    "POST /api/hosts/vm1/test": () => [200, tested],
    "POST /api/hosts/vm1/reconnect": () => [200, vm1],
  });
  const port = ["--port", String(s.port)];
  try {
    const added = await xenonet(["hosts", "add", "vm1", "lab@127.0.0.1", "--ssh-port", "60851", "--identity", "~/lab/console_ed25519", "--known-hosts", "~/lab/known_hosts", "--host-key-alias", "xenonet-lab-vm1", "--xenonet-port", "47841", "--command", "/home/lab/.local/bin/xenonet", "--since", "12h", "--detail", "--start", ...port]);
    assert.equal(added.code, 0, added.stderr);
    assert.deepEqual(s.seen.find((x) => x.method === "POST" && x.path === "/api/hosts").body, { id: "vm1", replace: false, target: "lab@127.0.0.1", sshPort: "60851", identity: "~/lab/console_ed25519", knownHosts: "~/lab/known_hosts", hostKeyAlias: "xenonet-lab-vm1", command: "/home/lab/.local/bin/xenonet", port: "47841", since: "12h", detail: true, start: true });
    assert.match(added.stdout, /✓ Added vm1 \(lab@127\.0\.0\.1:60851\)/);
    assert.match(added.stdout, /The first time, check vm1's host key in a terminal: ssh -F ~\/'Library\/Application Support\/XenoNet Dev\/ssh_config' vm1/);
    await xenonet(["hosts", "add", "xenonet", "--local", "--xenonet-port", "47840", "--replace", ...port]);
    assert.deepEqual(s.seen.filter((x) => x.method === "POST" && x.path === "/api/hosts").at(-1).body, { id: "xenonet", replace: true, transport: "local", port: "47840" });

    assert.match((await xenonet(["hosts", "remove", "vm1", "--yes", ...port])).stdout, /✓ Removed vm1, and where you placed 4 of its nodes/);
    assert.match((await xenonet(["hosts", "remove", "vm1", "--no-forget", ...port])).stdout, /✓ Removed vm1\. /);
    assert.deepEqual(s.seen.filter((x) => x.method === "DELETE").map((x) => x.search), ["?forget=1", ""]);

    const ok = await xenonet(["hosts", "test", "vm1", ...port]);
    assert.equal(ok.code, 0);
    assert.match(ok.stdout, /✓ vm1 answered in 1\.2 s: XenoNet 0\.4\.0 on linux, made-up data, calling itself lab/);
    assert.match(ok.stdout, /✓ a map of 32 places and sessions, 14 of them sessions, 9 live/);
    assert.match(ok.stdout, /✓ its clock: within 2 s of this one's/);
    tested = { id: "vm1", ok: false, ms: 900, state: "error", error: mac1.error };
    const bad = await xenonet(["hosts", "test", "vm1", ...port]);
    assert.equal(bad.code, 1);
    assert.match(bad.stdout.replace(/\s+/g, " "), /✗ vm1: ssh doesn't know mac1's host key, or it changed ↳ Check it and accept it once/);
    assert.match((await xenonet(["hosts", "reconnect", "vm1", ...port])).stdout, /✓ Connecting to vm1 now\./);

    for (const args of [["hosts", "add"], ["hosts", "remove"], ["hosts", "test"], ["hosts", "fly"]]) assert.equal((await xenonet([...args, ...port])).code, 2, args.join(" "));
  } finally {
    s.close();
  }
});

test("xenonet relay refuses a window it can't read, and a port that isn't one", async () => {
  const since = await xenonet(["relay", "--since", "forever", "--port", "1"]);
  assert.deepEqual([since.code, since.stderr.trim()], [2, "xenonet: --since is a number and m, h or d, such as 7d, 12h or 90m, not forever"]);
  assert.equal((await xenonet(["relay", "--port", "99999"])).code, 2);
  // Nothing to relay: a bye that says why, and the code for it.
  const free = await new Promise((resolve) => {
    const probe = http.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
  const none = await xenonet(["relay", "--port", String(free)]);
  assert.equal(none.code, 3);
  assert.deepEqual(JSON.parse(none.stdout), { t: "bye", code: "no-xenonet", message: `no XenoNet answers on 127.0.0.1:${free} on this machine` });
});
