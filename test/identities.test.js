import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { claudeAccount, chatgptAccount, parseGhHosts, parseSshAliases, parseRemote, parseGitConfig, pushAccount, noteSignin, accountAt, linkIdentities, emptySaved, IdentityWatch } from "../core/identities.js";
import { gitAvailable } from "../core/git.js";

// Every identity here is made up.
// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-id-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));
const ACCT = "0190d3e0-0000-7000-a000-000000000001";
const ORG = "0190d3e0-0000-7000-b000-000000000001";
const jwt = (claims) => ["e30", Buffer.from(JSON.stringify(claims)).toString("base64url"), "sig"].join(".");
const CHATGPT_AUTH = {
  auth_mode: "chatgpt",
  OPENAI_API_KEY: null,
  tokens: {
    id_token: jwt({ email: "nova@example.com", name: "Nova Q. Example", auth_time: 1_700_000_000, "https://api.openai.com/auth": { chatgpt_account_id: "ws-demo", chatgpt_plan_type: "plus" } }),
    access_token: "secret-access-token",
    refresh_token: "secret-refresh-token",
    account_id: "ws-demo",
  },
};

test("the Claude account comes from oauthAccount, without the name fields", () => {
  const a = claudeAccount({ oauthAccount: { accountUuid: ACCT, organizationUuid: ORG, emailAddress: "nova@example.com", organizationName: "nova@example.com's Organization", organizationType: "claude_max", displayName: "Nova", fullName: "Nova Q. Example" } });
  assert.deepEqual(a, { id: `claude:${ACCT}:${ORG}`, kind: "claude", handle: "nova@example.com", detail: { plan: "Max", org: null } });
  assert.ok(!JSON.stringify(a).includes("Nova Q."));
  // A team organization's name says something; a personal one's is the email.
  assert.equal(claudeAccount({ oauthAccount: { accountUuid: ACCT, organizationUuid: ORG, organizationName: "Atlas", organizationType: "claude_team" } }).detail.org, "Atlas");
  assert.equal(claudeAccount({}), null);
});

test("the ChatGPT account comes from the ID token's claims, and no token is kept", () => {
  const a = chatgptAccount(CHATGPT_AUTH);
  assert.deepEqual(a, { id: "chatgpt:ws-demo", kind: "chatgpt", handle: "nova@example.com", detail: { plan: "plus" }, since: 1_700_000_000_000 });
  const text = JSON.stringify(a);
  for (const secret of ["secret-access-token", "secret-refresh-token", "Nova Q.", CHATGPT_AUTH.tokens.id_token]) assert.ok(!text.includes(secret), secret);
  assert.deepEqual(chatgptAccount({ auth_mode: "apikey", OPENAI_API_KEY: "sk-made-up" }), { id: "openai:api-key", kind: "openai", handle: null, detail: { plan: null } });
  assert.equal(chatgptAccount({ tokens: { id_token: "not a jwt" } }), null);
});

test("gh's hosts.yml gives each host's users and the active one, never a token", () => {
  const hosts = parseGhHosts(`github.com:
    git_protocol: https
    users:
        nova-dev:
        orbitworks:
            oauth_token: gho_madeup123
    user: nova-dev
ghe.atlas.example:
    oauth_token: gho_older_format
    user: nova
`);
  assert.deepEqual(hosts, [
    { host: "github.com", users: ["nova-dev", "orbitworks"], active: "nova-dev" },
    { host: "ghe.atlas.example", users: ["nova"], active: "nova" },
  ]);
  assert.ok(!JSON.stringify(hosts).includes("gho_"));
});

test("remotes resolve to a host, owner and repo, through SSH aliases, dropping any token", () => {
  const aliases = parseSshAliases("Host gh-orbit\n  HostName github.com\n  IdentityFile ~/.ssh/orbit\nHost *\n  AddKeysToAgent yes\n");
  assert.deepEqual([...aliases], [["gh-orbit", "github.com"]]);
  const r = (url) => parseRemote(url, aliases);
  assert.deepEqual(r("git@github.com:atlas-co/atlas-api.git"), { host: "github.com", owner: "atlas-co", repo: "atlas-api", https: false, user: null });
  assert.deepEqual(r("git@gh-orbit:orbitworks/pathfinder.git"), { host: "github.com", owner: "orbitworks", repo: "pathfinder", https: false, user: null });
  assert.deepEqual(r("https://github.com/atlas-co/atlas-web"), { host: "github.com", owner: "atlas-co", repo: "atlas-web", https: true, user: null });
  assert.deepEqual(r("https://orbitworks:ghp_madeup@github.com/orbitworks/sim.git"), { host: "github.com", owner: "orbitworks", repo: "sim", https: true, user: "orbitworks" });
  assert.equal(r("ssh://git@github.com:22/nova-dev/ledger.git").owner, "nova-dev");
  assert.equal(r("C:/repos/local"), null);
  assert.equal(r("../elsewhere"), null);
});

test("a push signs in as the user git config names, else as gh's active account", () => {
  const cfg = parseGitConfig(
    [
      "user.name Nova",
      "user.email nova@example.com",
      "credential.https://github.com.helper",
      "credential.https://github.com.helper !/opt/homebrew/bin/gh auth git-credential",
      "user.email hello@orbitworks.example",
    ].join("\n"),
  );
  assert.equal(cfg.email, "hello@orbitworks.example");
  assert.deepEqual(pushAccount("https://github.com/atlas-co/atlas-api.git", cfg), { host: "github.com", user: null, how: "gh" });
  const pinned = parseGitConfig(["credential.helper osxkeychain", "credential.https://github.com.username orbitworks"].join("\n"));
  assert.deepEqual(pushAccount("https://github.com/orbitworks/pathfinder", pinned), { host: "github.com", user: "orbitworks", how: "git config" });
  // An empty helper clears the list; one for another host doesn't count.
  const cleared = parseGitConfig(["credential.helper !gh auth git-credential", "credential.helper", "credential.https://gitlab.example.helper !gh auth git-credential"].join("\n"));
  assert.equal(pushAccount("https://github.com/a/b", cleared), null);
  assert.equal(pushAccount("git@github.com:a/b.git", pinned), null);
});

test("the sign-in timeline gives each session the account signed in when it ran", () => {
  const H = 3600e3;
  const spans = [];
  assert.equal(noteSignin(spans, "claude:a", 10 * H), true);
  assert.equal(noteSignin(spans, "claude:a", 10 * H + 60e3), false);
  noteSignin(spans, "claude:a", 12 * H);
  noteSignin(spans, "claude:b", 20 * H);
  const now = { id: "claude:b", since: null };
  assert.deepEqual(accountAt(spans, now, 11 * H), { id: "claude:a", confidence: "likely", via: "signed in when it ran" });
  assert.equal(accountAt(spans, now, 21 * H).id, "claude:b");
  // Between the two, nobody knows; before XenoNet saw anything, neither.
  assert.equal(accountAt(spans, now, 15 * H), null);
  assert.equal(accountAt(spans, now, 1 * H), null);
  // With one account ever seen, everything is assumed to be it.
  assert.deepEqual(accountAt([{ id: "x", from: 10 * H, to: 12 * H }], { id: "x" }, 1 * H), { id: "x", confidence: "assumed", via: "the only account XenoNet has seen" });
  // Codex says when you signed in, which covers sessions before XenoNet saw it.
  assert.equal(accountAt([{ id: "c", from: 30 * H, to: 30 * H }], { id: "c", since: 5 * H }, 6 * H).confidence, "likely");
});

// A small graph: a group of two repos owned by orbitworks, a folder in it
// with no repo, an atlas-co repo, and sessions in each.
function world() {
  const P = (key, extra = {}) => ({ id: `project:${key}`, kind: "project", key, name: key.split("/").pop(), path: `/p/${key}`, ...extra });
  const S = (id, app, extra = {}) => ({ id, kind: "session", app, updatedAt: 50 * 3600e3, ...extra });
  const nodes = [
    { id: "core", kind: "core" },
    P("labs", { group: true }),
    P("labs/geo", { git: { origin: "git@gh-orbit:orbitworks/geo.git" } }),
    P("labs/sim", { git: { origin: "https://github.com/orbitworks/sim" } }),
    P("labs/notes"),
    P("atlas-api", { git: { origin: "https://github.com/atlas-co/atlas-api.git" } }),
    S("claude:1", "claude", { claudeAccount: { account: ACCT, org: ORG } }),
    S("claude:2", "claude"),
    S("claude:3", "claude"),
    S("codex:1", "codex"),
    S("codex:2", "codex", { provider: "ollama" }),
  ];
  const links = [
    ["core", "project:labs"], ["project:labs", "project:labs/geo"], ["project:labs", "project:labs/sim"], ["project:labs", "project:labs/notes"], ["core", "project:atlas-api"],
    ["project:atlas-api", "claude:1"], ["project:labs/geo", "claude:2"], ["project:labs/notes", "claude:3"], ["project:atlas-api", "codex:1"], ["project:labs/sim", "codex:2"],
  ].map(([source, target]) => ({ source, target, kind: "x" }));
  return { nodes, links };
}
const ACCOUNTS = {
  claude: { id: `claude:${ACCT}:${ORG}`, kind: "claude", handle: "nova@example.com", detail: { plan: "Max", org: null } },
  chatgpt: { id: "chatgpt:ws-demo", kind: "chatgpt", handle: "nova@example.com", detail: { plan: "plus" }, since: 10 * 3600e3 },
  gh: [{ host: "github.com", users: ["nova-dev", "OrbitWorks"], active: "nova-dev" }],
  ssh: new Map([["gh-orbit", "github.com"]]),
};
const REPOS = new Map([
  ["labs/geo", { name: "Orbit", email: "hello@orbitworks.example", push: null }],
  ["labs/sim", { name: "Orbit", email: "hello@orbitworks.example", push: { host: "github.com", user: null, how: "gh" } }],
  ["atlas-api", { name: "Nova", email: "nova@atlas.example", push: { host: "github.com", user: "nova-dev", how: "git config" } }],
]);
const node = (g, id) => g.nodes.find((n) => n.id === id);

test("sessions link to the account they ran as, with how sure that is", () => {
  const g = world();
  const owners = new Map([["claude:2", { account: ACCT, org: "0190d3e0-0000-7000-b000-0000000000ff" }]]);
  linkIdentities(g, { accounts: ACCOUNTS, repos: REPOS, owners, now: 60 * 3600e3 });
  assert.deepEqual(node(g, "claude:1").ranAs, { id: `claude:${ACCT}:${ORG}`, confidence: "exact", via: "Claude desktop's session folder" });
  assert.deepEqual(node(g, "claude:2").ranAs, { id: `claude:${ACCT}:0190d3e0-0000-7000-b000-0000000000ff`, confidence: "exact", via: "its transcript" });
  // Two Claude accounts are known, and XenoNet only saw this one from 60h on.
  assert.equal(node(g, "claude:3").ranAs, null);
  assert.equal(node(g, "codex:1").ranAs.confidence, "likely");
  assert.equal(node(g, "codex:2").ranAs, null, "another provider's thread isn't the ChatGPT account's");
});

test("projects link to their owner, who pushes and who commits; groups and folders inherit the owner", () => {
  const g = world();
  linkIdentities(g, { accounts: ACCOUNTS, repos: REPOS, now: 60 * 3600e3 });
  const links = (key) => Object.fromEntries(node(g, `project:${key}`).identities.map((l) => [l.role, `${l.id} ${l.confidence}`]));
  // gh's spelling of a login wins, so both remotes reach the signed-in account.
  assert.deepEqual(links("labs/geo"), { owner: "github:github.com/orbitworks exact", "commits-as": `${node(g, "project:labs/geo").identities[1].id} exact` });
  assert.equal(links("labs/sim")["pushes-as"], "github:github.com/nova-dev likely");
  assert.equal(links("atlas-api")["pushes-as"], "github:github.com/nova-dev exact");
  assert.equal(links("labs").owner, "github:github.com/orbitworks likely");
  assert.equal(node(g, "project:labs/notes").identities[0].via, "its group, labs");
  const orbit = g.identities.find((i) => i.id === "github:github.com/orbitworks");
  assert.equal(orbit.handle, "OrbitWorks");
  assert.equal(orbit.signedIn, true);
  // labs, its three projects, and the sessions in them.
  assert.deepEqual([orbit.projects, orbit.sessions], [4, 3]);
  const atlas = g.identities.find((i) => i.id === "github:github.com/atlas-co");
  assert.deepEqual([atlas.signedIn, atlas.sources], [false, ["remote URLs"]]);
  // git identities are keyed by a hash, never the email.
  assert.ok(g.identities.filter((i) => i.kind === "git").every((i) => /^git:[0-9a-f]{16}$/.test(i.id)));
});

test("your pins and names win, and XenoNet remembers an account after you switch", () => {
  const saved = emptySaved();
  saved.pins["project:labs/notes"] = { identity: "github:github.com/nova-dev", at: 1 };
  saved.pins["claude:3"] = { identity: `claude:${ACCT}:${ORG}`, at: 1 };
  saved.names["github:github.com/orbitworks"] = "orbit";
  const g = world();
  const { dirty } = linkIdentities(g, { accounts: ACCOUNTS, repos: REPOS, saved, now: 60 * 3600e3 });
  assert.equal(dirty, true);
  assert.deepEqual(node(g, "project:labs/notes").identities, [{ id: "github:github.com/nova-dev", role: "owner", confidence: "yours", via: "your pin" }]);
  assert.equal(node(g, "claude:3").ranAs.confidence, "yours");
  assert.equal(g.identities.find((i) => i.id === "github:github.com/orbitworks").name, "orbit");

  // Signed out of Claude: the account keeps its email, and the timeline
  // still gives its sessions to it.
  const later = world();
  linkIdentities(later, { accounts: { ...ACCOUNTS, claude: null }, repos: REPOS, saved, now: 70 * 3600e3 });
  const claude = later.identities.find((i) => i.id === `claude:${ACCT}:${ORG}`);
  assert.deepEqual([claude.handle, claude.signedIn], ["nova@example.com", false]);
  assert.ok(claude.firstSeen <= 60 * 3600e3);
});

test("IdentityWatch reads a home folder's sign-ins and each repo's git identity, includeIf and all", { skip: !gitAvailable() }, async () => {
  const home = tmp();
  const w = (rel, text) => (fs.mkdirSync(path.dirname(path.join(home, rel)), { recursive: true }), fs.writeFileSync(path.join(home, rel), text));
  w(".claude.json", JSON.stringify({ oauthAccount: { accountUuid: ACCT, organizationUuid: ORG, emailAddress: "nova@example.com", organizationType: "claude_pro" } }));
  w(".codex/auth.json", JSON.stringify(CHATGPT_AUTH));
  w(".config/gh/hosts.yml", "github.com:\n    users:\n        nova-dev:\n    user: nova-dev\n");
  w(".gitconfig", `[user]\n\tname = Nova\n\temail = nova@example.com\n[includeIf "gitdir:${home.replace(/\\/g, "/")}/Projects/orbit/"]\n\tpath = ~/.gitconfig-orbit\n`);
  w(".gitconfig-orbit", "[user]\n\temail = hello@orbitworks.example\n[credential \"https://github.com\"]\n\tusername = orbitworks\n");
  const repo = (rel, origin) => {
    const dir = path.join(home, rel);
    fs.mkdirSync(dir, { recursive: true });
    execFileSync("git", ["init", "-q", dir], { env: { ...process.env, HOME: home, GIT_CONFIG_NOSYSTEM: "1" } });
    execFileSync("git", ["-C", dir, "remote", "add", "origin", origin]);
    return { key: rel, path: dir, role: "project", git: { origin } };
  };
  const projects = [repo("Projects/ledger", "https://github.com/nova-dev/ledger.git"), repo("Projects/orbit/sim", "https://github.com/orbitworks/sim.git")];
  const transcripts = path.join(home, ".claude", "projects", "-p");
  w(".claude/projects/-p/s1.jsonl", [JSON.stringify({ type: "user" }), JSON.stringify({ type: "bridge-session", ownerAccountUuid: ACCT, ownerOrganizationUuid: ORG })].join("\n") + "\n");

  const watch = new IdentityWatch({ home, claudeConfigFile: path.join(home, ".claude.json"), codexHome: path.join(home, ".codex"), ghConfigDir: path.join(home, ".config", "gh"), claudeProjectsDir: path.dirname(transcripts) });
  const acc = watch.accounts();
  assert.equal(acc.claude.detail.plan, "Pro");
  assert.equal(acc.chatgpt.id, "chatgpt:ws-demo");
  assert.equal(await watch.refreshRepos(projects), true);
  assert.equal(await watch.refreshRepos(projects), false, "nothing changed, so nothing is read again");
  assert.deepEqual(watch.repos.get("Projects/ledger").value, { name: "Nova", email: "nova@example.com", push: null });
  assert.deepEqual(watch.repos.get("Projects/orbit/sim").value, { name: "Nova", email: "hello@orbitworks.example", push: { host: "github.com", user: "orbitworks", how: "git config" } });
  assert.deepEqual(watch.transcriptOwner("claude:s1"), { account: ACCT, org: ORG });
  assert.equal(watch.transcriptOwner("claude:missing"), null);

  // A changed include file is read again.
  await new Promise((r) => setTimeout(r, 20));
  w(".gitconfig-orbit", "[user]\n\temail = hi@orbitworks.example\n");
  assert.equal(await watch.refreshRepos(projects), true);
  assert.equal(watch.repos.get("Projects/orbit/sim").value.email, "hi@orbitworks.example");
});

test("a transcript that can't be read leaves no file open", { skip: process.platform === "win32" }, () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-owner-"));
  const projects = path.join(home, ".claude", "projects");
  // A folder where a transcript should be: it opens, but reading it fails.
  fs.mkdirSync(path.join(projects, "-p", "s9.jsonl"), { recursive: true });
  const watch = new IdentityWatch({ home, claudeConfigFile: path.join(home, ".claude.json"), codexHome: path.join(home, ".codex"), ghConfigDir: path.join(home, ".config", "gh"), claudeProjectsDir: projects });
  const open = () => fs.readdirSync("/dev/fd").length;
  const before = open();
  assert.equal(watch.transcriptOwner("claude:s9"), null);
  assert.equal(open(), before);
  fs.rmSync(home, { recursive: true, force: true });
});

test("a source that can't be read says nothing, and the watch says which one, until it's fixed", { skip: !gitAvailable() }, async () => {
  const home = tmp();
  const w = (rel, text, mtime) => {
    fs.mkdirSync(path.dirname(path.join(home, rel)), { recursive: true });
    fs.writeFileSync(path.join(home, rel), text);
    if (mtime) fs.utimesSync(path.join(home, rel), mtime / 1000, mtime / 1000);
  };
  // Half written, with a made-up token in what's there.
  w(".codex/auth.json", '{"tokens": {"access_token": "secret-access-token" half written', Date.parse("2026-10-01T00:00:00Z"));
  const watch = new IdentityWatch({ home, claudeConfigFile: path.join(home, ".claude.json"), codexHome: path.join(home, ".codex"), ghConfigDir: path.join(home, ".config", "gh"), claudeProjectsDir: path.join(home, ".claude", "projects") });
  assert.equal(watch.accounts().chatgpt, null);
  assert.equal(watch.accounts().claude, null, "no ~/.claude.json: nothing, and no problem");
  let h = watch.health();
  assert.equal(h.state, "degraded");
  assert.equal(h.lastError.message.replace(/\\/g, "/"), "~/.codex/auth.json: isn't valid JSON (line 1, column 51)", "where it breaks, never the text");
  assert.equal(h.counters.errors, 1, "said once, however often it's read");
  assert.equal(h.counters.fileReads, 4, "and not read again until it changes");

  w(".codex/auth.json", JSON.stringify(CHATGPT_AUTH), Date.parse("2026-10-02T00:00:00Z"));
  assert.equal(watch.accounts().chatgpt.id, "chatgpt:ws-demo");
  assert.equal(watch.health().state, "running");

  // A repo git can't read: skipped, and tried again on the next pass.
  const projects = [{ key: "gone", path: path.join(home, "Projects", "gone"), role: "project", git: { origin: "https://github.com/nova-dev/gone.git" } }];
  assert.equal(await watch.refreshRepos(projects), false);
  assert.equal(watch.health().state, "degraded");
  assert.match(watch.lastError.message.replace(/\\/g, "/"), /git couldn't read the config of ~\/Projects\/gone/);
  await watch.refreshRepos(projects);
  assert.equal(watch.health().counters.repoReads, 2, "tried again");
  assert.equal(watch.repos.size, 0);
  assert.equal(await watch.refreshRepos([]), false);
  assert.equal(watch.health().state, "running", "a repo that's gone takes its problem with it");
});
