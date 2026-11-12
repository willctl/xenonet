// Xenolith in the lab: the checks of phase 6's commit 27, which `npm run lab -- check` runs
// after the transport's. vm1 curates a space and vm2 joins it in own mode, by `xenolith invite`
// on vm1 and `xenolith join` on vm2 over xenolith/join/1; a page and a note go between them; vm2
// is offline, then catches up; a third VM, when the lab has one, fills vm2 from its holder copy
// with vm1 stopped; shared mode over XenoNet's forward, once the owner has the VMs in a network
// and Xenolith's grant can be minted; and the cutover's rehearsal (phase 6 revision 33g): a 1.x
// curator and satellite with a queued note, 2.0's `upgrade --run`, `upgrade --undo`, and 1.x
// syncing again.
//
// Xenolith is built in each VM from the checkout lab.mjs pushed there, as XenoNet is, and 1.10.0
// from its tag (xenolith.sh). Each lab Xenolith has its own home and folder in the VM and runs as
// a transient user service; nothing is installed.
//
// A join's code is typed by whoever joins. Here the lab stands in for that person: it carries
// the code from vm1's invite to vm2's join on stdin, never in an argument or its output, as it
// carries a VM's sudo password. When the invite is refused the check says what the owner runs.
//
// lab.mjs gives each check `ctx`: the transport's (transport.mjs), with `x`, Xenolith's steps in
// the VMs (`xenolithOps` below). Everything else here is pure, so the tests can check it.
import { randomBytes } from "node:crypto";
import { OWNER as NET_OWNER, sharedNetwork } from "./transport.mjs";

// The pieces the checks wait on, by the subject of the commit that lands each on main, which the
// lab looks for in the history the VM's checkout was built from. The last isn't built: the commit
// that lets XenoNet mint Xenolith's grant (key(id) and forward(xenolith/sync/2)) names "Xenolith's
// grant" in its subject.
export const XLANDS = {
  upgrade: { where: "phase 6, the upgrade gate", subject: /^feat\(xenolith\)!?: .*\bupgrade\b/i },
  "sync/2": { where: "phase 6 commit 13", subject: /^feat\(xenolith\)!?: .*\bsync\/2\b/i },
  "join/1": { where: "phase 6 commit 14", subject: /^feat\(xenolith\)!?: .*\bjoin\/1\b/i },
  holders: { where: "phase 6 commit 15", subject: /^feat\(xenolith\)!?: .*\b(holders?|holding)\b/i },
  "shared mode": { where: "phase 6 commit 16", subject: /^feat\([^)]*\bxenolith\b[^)]*\)!?: .*\bshared mode\b/i },
  "Xenolith's grant": { where: "not built yet: XenoNet minting Xenolith's grant", subject: /^feat\([^)]*\)!?: .*\bXenolith'?s grant\b/i },
};

export function xmissing(needs, subjects) {
  return needs.filter((n) => !subjects.some((s) => XLANDS[n].subject.test(s)));
}

export const xwaitsOn = (pieces) => `waits on ${pieces.map((p) => `${p} (${XLANDS[p].where})`).join(", ")}`;

export const SPACE = "project:atlas-api";
export const SYNC_PORT = 47851;
// 1.10.0, the last 1.x, for the rehearsal: its tag in this checkout, pushed to this branch in vm1.
export const OLD_TAG = "xenolith-v1.10.0";
export const OLD_BRANCH = "xenolith-1x";
// The lab's Xenoliths, by VM: vm1 curates, vm2 and vm3 are members.
export const NAMES = { vm1: "curator", vm2: "member", vm3: "holder" };
export const REHEARSAL = ["built", "1x-synced", "queued", "upgraded", "rolledback", "1x-again"];

const XBIN = "~/xenonet/xenolith/rust/target/debug/xenolith";
const as = (vm) => `XENOLITH_HOME=~/.local/share/xenonet-lab/xenolith/${NAMES[vm]}/home XENOLITH_DIR=~/.local/share/xenonet-lab/xenolith/${NAMES[vm]}/wiki ${XBIN}`;

// What the owner runs, which the lab can't.
export const XOWNER = {
  join: `join vm2 to vm1's ${SPACE} yourself: in vm1 (limactl shell xenonet-vm1), ${as("vm1")} invite ${SPACE} --level write; in vm2, ${as("vm2")} join <the invite> --at <vm1's lima0 address>:${SYNC_PORT}, and type the code vm1 showed`,
  grant: "give the Xenolith in vm1 and in vm2 its grant from XenoNet (key(id) and forward(xenolith/sync/2)): approve it for each in the Mac's XenoNet window, then pipe it into xenolith sync grant in that VM",
  share: `share vm1's ${SPACE} into the lab's network: the curator drafts the entry (${as("vm1")} spaces draft), a device of yours signs it with your presence in the Mac's XenoNet window, and the curator sequences it (spaces sequence)`,
  old: `fetch Xenolith ${OLD_TAG.slice(10)}'s tag into this checkout (git fetch origin tag ${OLD_TAG}), then run npm run lab -- check --only xenolith-rehearsal`,
};

export const nonce = (what) => `lab-${what}-${randomBytes(6).toString("hex")}`;
const page = (title, body) => `---\ntitle: ${title}\ntype: project\n---\n\n# ${title}\n\n${body}\n`;
const dir = (space) => `spaces/${space.replace(":", "/")}`;
const pageRel = (slug) => `${dir(SPACE)}/pages/${slug}.md`;
const inboxRel = `${dir(SPACE)}/inbox`;

const ok = (detail) => ({ status: "ok", detail });
const fail = (detail) => ({ status: "FAIL", detail });
const owner = (detail) => ({ status: "owner", detail });
const skip = (detail) => ({ status: "skip", detail });
const lastLine = (s) => String(s ?? "").trim().split("\n").pop() ?? "";

// A path inside a lab Xenolith's folder, never outside it.
export function folderPath(rel) {
  if (!/^[A-Za-z0-9._/-]+$/.test(rel) || rel.split("/").some((p) => p === ".." || p === "") || rel.startsWith("/")) throw new Error(`not a path in the folder: ${rel}`);
  return rel;
}

// The rehearsal's verdict from its `stage=<id> ok|FAIL <detail>` lines.
export function rehearsalVerdict(stdout) {
  const lines = String(stdout ?? "").split("\n").map((l) => l.match(/^stage=(\S+) (ok|FAIL) ?(.*)$/)).filter(Boolean);
  const bad = lines.find((m) => m[2] === "FAIL");
  if (bad) return fail(`${bad[1]}: ${bad[3]}`);
  const done = lines.map((m) => m[1]);
  const left = REHEARSAL.filter((s) => !done.includes(s));
  if (left.length) return fail(`the rehearsal stopped before ${left[0]}${done.length ? `, after ${done.at(-1)}` : ""}`);
  return ok(lines.map((m) => m[3]).join("; "));
}

// A lab Xenolith's sync status (`xenolith sync status --json`), or null.
async function status(ctx, vm) {
  const r = await ctx.x.cli(vm, NAMES[vm], ["sync", "status", "--json"]);
  try {
    return r.code === 0 ? (JSON.parse(r.out)?.sync ?? null) : null;
  } catch {
    return null;
  }
}

const spaceIn = (s, address = SPACE) => (s?.spaces ?? []).find((x) => x?.address === address) ?? null;

// The time, from ctx when it keeps its own (the tests), else the clock.
const clock = (ctx) => ctx.now?.() ?? Date.now();

async function poll(ctx, ms, fn) {
  const until = clock(ctx) + ms;
  for (;;) {
    const v = await fn();
    if (v || clock(ctx) >= until) return v;
    await ctx.sleep(1000);
  }
}

async function built(ctx, vms) {
  for (const vm of vms) {
    const b = await ctx.x.build(vm);
    if (!b.ok) return `${vm}: Xenolith didn't build (${b.why})`;
  }
  return null;
}

// Whether vm2 (and vm3) are members of vm1's space already, with every member's endpoint known.
async function joined(ctx, members) {
  const cur = spaceIn(await status(ctx, "vm1"));
  if (cur?.role !== "curator") return null;
  for (const vm of members) {
    const s = await status(ctx, vm);
    const mine = spaceIn(s);
    if (mine?.role !== "member" || mine.curator !== cur.curator || !cur.members?.some((m) => m.key === s.key)) return null;
  }
  return cur;
}

// vm1 curating SPACE with a page, and each of `members` joined to it by an invite and its code:
// the result, or {why} (a FAIL) or {owner} (what the owner runs). Kept for the run's later
// checks; `fresh` starts them all again.
async function setup(ctx, members, { fresh = false } = {}) {
  const key = members.join(",");
  ctx.xjoined ??= {};
  if (!fresh && ctx.xjoined[key]) return ctx.xjoined[key];
  const broke = await built(ctx, ["vm1", ...members]);
  if (broke) return { why: broke };
  if (!fresh) {
    for (const vm of ["vm1", ...members]) await ctx.x.start(vm, NAMES[vm]);
    const cur = await joined(ctx, members);
    if (cur) return (ctx.xjoined[key] = { space: cur.id });
  }
  const text = page("Atlas API in the lab", `Made by the lab, ${nonce("page")}.`);
  await ctx.x.reset("vm1", NAMES.vm1, { space: SPACE, page: text });
  for (const vm of members) await ctx.x.reset(vm, NAMES[vm], {});
  const founded = await poll(ctx, 30000, async () => spaceIn(await status(ctx, "vm1")));
  if (!founded) return { why: `vm1 didn't found ${SPACE} in 30 s` };
  const addr = (await ctx.lanAddrs("vm1")).find((a) => !a.includes(":"));
  if (!addr) return { why: "vm1 has no IPv4 address on the lab's LAN" };
  for (const vm of members) {
    const inv = await ctx.x.cli("vm1", NAMES.vm1, ["invite", SPACE, "--level", vm === "vm2" ? "write" : "read", "--json"]);
    if (inv.code !== 0) return /owner key|not answering|no longer running/i.test(inv.err) ? { owner: XOWNER.join } : { why: `vm1's invite was refused: ${lastLine(inv.err)}` };
    let offer;
    try {
      offer = JSON.parse(inv.out);
    } catch {
      return { why: "vm1's invite wasn't JSON" };
    }
    const j = await ctx.x.cli(vm, NAMES[vm], ["join", offer.invite, "--at", `${addr}:${SYNC_PORT}`, "--json"], `${offer.code}\n`);
    if (j.code !== 0) return { why: `${vm}'s join was refused: ${lastLine(j.err)}` };
    let out;
    try {
      out = JSON.parse(j.out);
    } catch {
      return { why: `${vm}'s join answer wasn't JSON` };
    }
    if (!(out.followed ?? []).some((f) => f.space === founded.id)) return { why: `${vm} joined but doesn't follow ${SPACE}` };
  }
  return (ctx.xjoined[key] = { space: founded.id });
}

const same = async (ctx, a, b, rel) => {
  const x = await ctx.x.read(a, NAMES[a], rel);
  return x != null && x === (await ctx.x.read(b, NAMES[b], rel));
};

// Each check: what it shows, the VMs and pieces it needs, what the owner sets up, what an agent
// uses instead when it can't run, what it changes in the VMs (and puts back), and its run.
export const XCHECKS = [
  {
    id: "xenolith-join",
    what: "vm2 joins vm1's space in own mode, by xenolith invite on vm1 and xenolith join on vm2, and vm1's page reaches it",
    vms: ["vm1", "vm2"],
    needs: ["sync/2", "join/1"],
    owner: XOWNER.join,
    standIn: "the lab carries the code from the invite to the join on stdin; crates/net/tests/join.rs and xenolith/test/join.mjs on loopback",
    changes: "a new lab Xenolith in vm1 (curating project:atlas-api) and in vm2, each its own home and folder, which stay for the next checks",
    run: async (ctx) => {
      const s = await setup(ctx, ["vm2"], { fresh: true });
      if (s.owner) return owner(s.owner);
      if (s.why) return fail(s.why);
      if (!(await poll(ctx, 60000, () => same(ctx, "vm1", "vm2", pageRel("lab"))))) return fail("vm1's page didn't reach vm2 in 60 s");
      const two = await status(ctx, "vm2");
      const member = spaceIn(await status(ctx, "vm1"))?.members?.find((m) => m.key === two?.key);
      if (member?.endpoint !== two?.endpointId || member?.level !== "write") return fail(`vm1's share for vm2 names ${member ? `${member.endpoint} at ${member.level}` : "nothing"}, not vm2's endpoint at write`);
      return ok(`vm2 joined at write over vm1:${SYNC_PORT}, and holds vm1's page`);
    },
  },
  {
    id: "xenolith-notes",
    what: "a note left on vm2 reaches vm1's inbox, stamped as vm2's key and never as the person, and the page vm1's curator files comes back to vm2",
    vms: ["vm1", "vm2"],
    needs: ["sync/2", "join/1"],
    owner: XOWNER.join,
    standIn: "xenolith/test/sync.mjs on loopback",
    changes: "a note in vm1's inbox and a page in its folder, which stay",
    run: async (ctx) => {
      const s = await setup(ctx, ["vm2"]);
      if (s.owner) return owner(s.owner);
      if (s.why) return fail(s.why);
      const text = nonce("note");
      const n = await ctx.x.cli("vm2", NAMES.vm2, ["note", text, "--body", "Left on vm2 by the lab.", "--space", SPACE, "--json"]);
      if (n.code !== 0) return fail(`vm2 refused the note: ${lastLine(n.err)}`);
      const notes = await poll(ctx, 30000, async () => {
        const found = await ctx.x.find("vm1", NAMES.vm1, inboxRel, text);
        return found.length ? found : null;
      });
      if (!notes) return fail("vm2's note didn't reach vm1's inbox in 30 s");
      const key = (await status(ctx, "vm2"))?.key;
      const note = notes[0];
      if (!/^transport: sync$/m.test(note)) return fail("the note in vm1's inbox didn't come by sync");
      if (!note.includes(`by: "key:${key}"`)) return fail(`the note in vm1's inbox isn't stamped as vm2's key (${key})`);
      if (!/^source: agent$/m.test(note)) return fail("a peer's note speaks as the person in vm1's inbox");
      // vm1's curator files it, as its model would: into a page of vm1's.
      await ctx.x.write("vm1", NAMES.vm1, pageRel("notes"), page("Lab notes", `Filed from vm2: ${text}.`));
      if (!(await poll(ctx, 30000, () => same(ctx, "vm1", "vm2", pageRel("notes"))))) return fail("the filed page didn't come back to vm2 in 30 s");
      return ok("vm2's note reached vm1's inbox as key:<vm2>, source agent; the filed page came back");
    },
  },
  {
    id: "xenolith-offline",
    what: "vm2's Xenolith stopped while vm1's page changes catches up when it's back; a note left on vm2 with vm1 stopped reaches vm1 when it's back",
    vms: ["vm1", "vm2"],
    needs: ["sync/2", "join/1"],
    owner: XOWNER.join,
    standIn: "xenolith/test/sync.mjs (partition then catch-up) on loopback",
    changes: "stops vm2's lab Xenolith, then vm1's, and starts both again",
    run: async (ctx) => {
      const s = await setup(ctx, ["vm2"]);
      if (s.owner) return owner(s.owner);
      if (s.why) return fail(s.why);
      try {
        await ctx.x.stop("vm2", NAMES.vm2);
        await ctx.x.write("vm1", NAMES.vm1, pageRel("offline"), page("While vm2 was away", nonce("offline")));
        await ctx.sleep(3000);
        await ctx.x.start("vm2", NAMES.vm2);
        if (!(await poll(ctx, 60000, () => same(ctx, "vm1", "vm2", pageRel("offline"))))) return fail("vm2 didn't catch up on vm1's page in 60 s");
        await ctx.x.stop("vm1", NAMES.vm1);
        const text = nonce("queued");
        const n = await ctx.x.cli("vm2", NAMES.vm2, ["note", text, "--body", "Left on vm2 while vm1 was away.", "--space", SPACE, "--json"]);
        if (n.code !== 0) return fail(`vm2 refused the note: ${lastLine(n.err)}`);
        await ctx.sleep(3000);
        await ctx.x.start("vm1", NAMES.vm1);
        const came = await poll(ctx, 60000, async () => (await ctx.x.find("vm1", NAMES.vm1, inboxRel, text)).length > 0);
        return came ? ok("vm2 caught up on the page; its note reached vm1 once vm1 was back") : fail("the note vm2 kept while vm1 was away didn't reach vm1 in 60 s");
      } finally {
        await ctx.x.start("vm1", NAMES.vm1);
        await ctx.x.start("vm2", NAMES.vm2);
      }
    },
  },
  {
    id: "xenolith-holder",
    what: "with vm1's Xenolith stopped, vm2 fills its gap from vm3's holder copy of the curator's pages",
    vms: ["vm1", "vm2", "vm3"],
    needs: ["sync/2", "join/1", "holders"],
    owner: XOWNER.join,
    standIn: "crates/net/tests/holders.rs (curator offline, B fills C) in xenolith/rust",
    changes: "a lab Xenolith in vm3 joined at read; stops vm2's, then vm1's, and starts both again",
    run: async (ctx) => {
      const s = await setup(ctx, ["vm2", "vm3"]);
      if (s.owner) return owner(s.owner);
      if (s.why) return fail(s.why);
      try {
        await ctx.x.stop("vm2", NAMES.vm2);
        await ctx.x.write("vm1", NAMES.vm1, pageRel("held"), page("Held for vm2", nonce("held")));
        if (!(await poll(ctx, 60000, () => same(ctx, "vm1", "vm3", pageRel("held"))))) return fail("vm3 didn't get vm1's page in 60 s");
        await ctx.x.stop("vm1", NAMES.vm1);
        await ctx.x.start("vm2", NAMES.vm2);
        const want = await ctx.x.read("vm3", NAMES.vm3, pageRel("held"));
        const got = await poll(ctx, 60000, async () => (await ctx.x.read("vm2", NAMES.vm2, pageRel("held"))) === want);
        return got ? ok("vm2 took the page from vm3 with vm1 stopped") : fail("vm2 didn't get the page from vm3 in 60 s with vm1 stopped");
      } finally {
        await ctx.x.start("vm1", NAMES.vm1);
        await ctx.x.start("vm2", NAMES.vm2);
      }
    },
  },
  {
    id: "xenolith-shared",
    what: "shared mode: with both VMs in a XenoNet network and Xenolith's grant in each, a space shared into the network reaches vm2 over XenoNet's forward",
    vms: ["vm1", "vm2"],
    needs: ["sync/2", "shared mode", "Xenolith's grant"],
    owner: `${NET_OWNER.join}; then ${XOWNER.grant}; then ${XOWNER.share}`,
    standIn: "crates/xenonet-service/tests/xenolith.rs (three services and three Xenoliths)",
    changes: "makes the lab's Xenoliths in vm1 and vm2 when the earlier checks haven't; nothing more than what the owner shares",
    run: async (ctx) => {
      if (!sharedNetwork(await ctx.state("vm1"), await ctx.state("vm2"))) return owner(NET_OWNER.join);
      const broke = await built(ctx, ["vm1", "vm2"]);
      if (broke) return fail(broke);
      // The lab's Xenoliths, made when the earlier checks haven't, for the owner to give them
      // their grants: vm1 curating the space.
      const st = {};
      for (const vm of ["vm1", "vm2"]) {
        await ctx.x.start(vm, NAMES[vm]);
        st[vm] = await status(ctx, vm);
        if (!st[vm]) {
          await ctx.x.reset(vm, NAMES[vm], vm === "vm1" ? { space: SPACE, page: page("Atlas API in the lab", "Shared into the lab's network.") } : {});
          st[vm] = await status(ctx, vm);
        }
      }
      const detached = ["vm1", "vm2"].filter((vm) => st[vm]?.xenonet?.mode !== "attached");
      if (detached.length) return owner(`${XOWNER.grant} (not attached: ${detached.join(", ")})`);
      const shared = (st.vm1?.spaces ?? []).find((x) => x.role === "curator" && x.networks?.length);
      if (!shared) return owner(XOWNER.share);
      const reached = await poll(ctx, 60000, async () => spaceIn(await status(ctx, "vm2"), shared.address)?.role === "member");
      if (!reached) return fail(`${shared.address}, shared into ${shared.networks.map((n) => n.network).join(", ")}, didn't reach vm2 in 60 s`);
      const bound = (await status(ctx, "vm2"))?.xenonet?.peers?.find((p) => p.key === st.vm1.key);
      return bound?.connected ? ok(`${shared.address} reached vm2 over the forward, vm1 bound as ${bound.machine}`) : fail("vm2 holds the space but vm1 isn't a bound, connected peer");
    },
  },
  {
    id: "xenolith-rehearsal",
    what: "the cutover's rehearsal: a 1.x curator and satellite with a queued note, 2.0's upgrade --run, then --undo, and 1.x syncs again",
    vms: ["vm1"],
    needs: ["upgrade"],
    owner: XOWNER.old,
    standIn: "the upgrade tests in xenolith/test/spaces.mjs (a 1.x satellite fixture, --run, --undo)",
    changes: "builds 1.10.0 in vm1 (~/xenolith-1x); two Xenoliths on vm1's 127.0.0.1, deleted afterwards",
    run: async (ctx) => {
      if (!(await ctx.x.hasRef(OLD_TAG))) return owner(XOWNER.old);
      const broke = await built(ctx, ["vm1"]);
      if (broke) return fail(broke);
      const old = await ctx.x.old("vm1");
      if (!old.ok) return fail(`vm1: Xenolith ${OLD_TAG.slice(10)} didn't build (${old.why})`);
      return rehearsalVerdict(await ctx.x.rehearse("vm1"));
    },
  },
];

// One check: "skip" without its VMs (saying what this checkout lacks), "wait" when a VM's build
// lacks a piece, then the check's own answer. A check that throws fails with the error.
export async function runXCheck(check, ctx) {
  const away = check.vms.filter((vm) => !ctx.has(vm));
  if (away.length) {
    const here = xmissing(check.needs, await ctx.subjects("here"));
    const vm3 = away.includes("vm3") && !away.some((v) => v !== "vm3");
    return skip(`${vm3 ? "the lab has no vm3 running" : `needs ${check.vms.join(" and ")} running`}${here.length ? `; this checkout ${xwaitsOn(here)}` : ""}`);
  }
  const lacking = new Set();
  for (const vm of check.vms) for (const piece of xmissing(check.needs, await ctx.subjects(vm))) lacking.add(piece);
  if (lacking.size) return { status: "wait", detail: `not in this build yet: ${xwaitsOn([...lacking])}` };
  try {
    return await check.run(ctx);
  } catch (err) {
    return fail(err?.message ?? String(err));
  }
}

// ---------- Xenolith's steps in the VMs ----------

// `ctx.x` from what lab.mjs gives: `guest(vm, step, vars, opts)` runs a step of xenolith.sh and
// gives {status, stdout, stderr}; `push(vm, ref, branch)` pushes a ref of this checkout to a
// branch in the VM's checkout (null when the ref isn't here); `hasRef(ref)`.
export function xenolithOps({ guest, push, hasRef }) {
  const must = async (vm, step, vars, what, opts) => {
    const r = await guest(vm, step, vars, opts);
    if (r.status !== 0) throw new Error(`${vm}: ${what} (${lastLine(r.stderr) || `exit ${r.status}`})`);
    return r;
  };
  const b64 = (s) => Buffer.from(String(s ?? ""), "utf8").toString("base64");
  return {
    hasRef,
    build: async (vm) => {
      const r = await guest(vm, "xbuild", {}, { timeout: 60 * 60000 });
      return r.status === 0 ? { ok: true } : { ok: false, why: lastLine(r.stderr) || `exit ${r.status}` };
    },
    reset: (vm, name, { space = "", page = "", bind = "" } = {}) => must(vm, "xreset", { NAME: name, SPACE: space, PAGE: b64(page), BIND: bind }, `the lab Xenolith ${name} didn't start`, { timeout: 120000 }),
    start: (vm, name) => must(vm, "xstart", { NAME: name }, `the lab Xenolith ${name} didn't start`, { timeout: 120000 }),
    stop: (vm, name) => guest(vm, "xstop", { NAME: name }),
    cli: async (vm, name, args, input = "") => {
      const r = await guest(vm, "xcli", { NAME: name, ARGS: JSON.stringify(args), INPUT: input }, { timeout: 120000 });
      return { code: r.status, out: r.stdout ?? "", err: r.stderr ?? "" };
    },
    read: async (vm, name, rel) => {
      const r = await guest(vm, "xread", { NAME: name, REL: folderPath(rel) });
      return r.status === 0 ? (r.stdout ?? "") : null;
    },
    write: (vm, name, rel, text) => must(vm, "xwrite", { NAME: name, REL: folderPath(rel), TEXT: b64(text) }, `couldn't write ${rel}`),
    find: async (vm, name, rel, needle) => {
      const r = await guest(vm, "xfind", { NAME: name, REL: folderPath(rel), NEEDLE: needle });
      try {
        return JSON.parse(r.stdout ?? "[]");
      } catch {
        return [];
      }
    },
    old: async (vm) => {
      if (!(await push(vm, OLD_TAG, OLD_BRANCH))) return { ok: false, why: `no ${OLD_TAG} in this checkout` };
      const r = await guest(vm, "xold", {}, { timeout: 60 * 60000 });
      return r.status === 0 ? { ok: true } : { ok: false, why: lastLine(r.stderr) || `exit ${r.status}` };
    },
    rehearse: async (vm) => (await guest(vm, "xrehearse", {}, { timeout: 10 * 60000 })).stdout ?? "",
  };
}
