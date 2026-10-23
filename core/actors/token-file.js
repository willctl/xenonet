// Where an actor keeps its token (docs/design/actors/interface.md, section
// 3, "How an actor gets its token"): a file only you can read (0600), named
// in XENONET_ACTOR_TOKEN_FILE, or the token itself in XENONET_ACTOR_TOKEN.
// Never an argument: any program you run can read the arguments of every
// other one with ps, and agents run as you. A file others can read is
// refused, as ssh refuses loose keys, and one isn't written inside a git
// working tree or the projects folder, where agents read freely and tokens
// get committed, unless you say so.
//
// Who can read a file is its mode bits on macOS and Linux, and its ACL on
// Windows, where XenoNet asks icacls and whoami (`run`, a command's output).
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { aclOf, userSid, othersIn, protectFile } from "../private-file.js";
export { aclOf, userSid, othersIn } from "../private-file.js";
const runSync = (file, args) => execFileSync(file, args, { encoding: "utf8", windowsHide: true, timeout: 10000 });
import { isToken, hashToken, TOKEN_GREP } from "./tokens.js";
import { pathKey, isUnder } from "../paths.js";

// The token in a file, when only you can read it.
export function readTokenFile(file, { platform = process.platform, run = runSync } = {}) {
  let st;
  try {
    st = fs.statSync(file);
  } catch {
    throw new Error(`there's no token file at ${file}`);
  }
  if (!st.isFile()) throw new Error(`${file} isn't a file`);
  if (platform === "win32") {
    let others;
    try {
      others = othersIn(aclOf(file, { run }), userSid({ run }));
    } catch {
      throw new Error(`XenoNet couldn't tell who else can read ${file}, so it won't use the token in it; keep it in a folder of your own, such as one in your user folder`);
    }
    if (others.length) throw new Error(`other users can read ${file}, so XenoNet won't use the token in it; make it yours alone with icacls "${file}" /inheritance:r /grant:r "%USERNAME%:F"`);
  } else if (st.mode & 0o077) throw new Error(`other users can read ${file}, so XenoNet won't use the token in it; make it yours alone with chmod 600`);
  const token = fs.readFileSync(file, "utf8").trim();
  if (!isToken(token)) throw new Error(`${file} doesn't hold an actor's token`);
  return token;
}

// The token an actor runs with, from its environment: { token, file }, or
// null when it has none.
export function tokenFromEnv(env = process.env, opts) {
  if (env.XENONET_ACTOR_TOKEN_FILE) return { token: readTokenFile(env.XENONET_ACTOR_TOKEN_FILE, opts), file: env.XENONET_ACTOR_TOKEN_FILE };
  if (env.XENONET_ACTOR_TOKEN) {
    const token = env.XENONET_ACTOR_TOKEN.trim();
    if (!isToken(token)) throw new Error("XENONET_ACTOR_TOKEN doesn't hold an actor's token");
    return { token, file: null };
  }
  return null;
}

// The git working tree holding a folder, or null.
export function gitTreeOf(dir) {
  for (let d = path.resolve(dir); ; d = path.dirname(d)) {
    if (fs.existsSync(path.join(d, ".git"))) return d;
    if (path.dirname(d) === d) return null;
  }
}

// Why a token shouldn't be written to `file`, or null when it may: a file
// that's there already, or one inside a git working tree or the projects
// folder. `force` says to go ahead anyway.
export function tokenFileRefusal(file, { force = false, projectsDir = null } = {}) {
  if (force) return null;
  const full = path.resolve(file);
  const tree = gitTreeOf(path.dirname(full));
  if (tree) return `${full} is inside a git working tree (${tree}), where agents read freely and tokens get committed. Pick a file elsewhere, such as ~/.config/<name>/xenonet.token, or pass --force`;
  if (projectsDir && isUnder(pathKey(full), pathKey(projectsDir))) return `${full} is inside the projects folder, where agents read freely. Pick a file elsewhere, such as ~/.config/<name>/xenonet.token, or pass --force`;
  if (fs.existsSync(full)) return `${full} is there already; pass --force to replace it`;
  return null;
}

// Writes a token to a file only you can read (0600, or on Windows an ACL
// with your account alone and nothing inherited), in a folder made only for
// you if it isn't there. Says where.
export function writeTokenFile(file, token, { platform = process.platform, run = runSync, ...opts } = {}) {
  if (!isToken(token)) throw new Error("that isn't an actor's token");
  const why = tokenFileRefusal(file, opts);
  if (why) throw new Error(why);
  const full = path.resolve(file);
  fs.mkdirSync(path.dirname(full), { recursive: true, mode: 0o700 });
  // On Windows the file is made new and empty, made yours alone (no
  // inherited entries, and your account's alone, which writing keeps), and
  // only then given the token. Elsewhere, a file that's there is made yours
  // alone before the token goes in.
  if (platform === "win32") {
    fs.rmSync(full, { force: true });
    fs.writeFileSync(full, "");
    try {
      protectFile(full, { platform, run });
    } catch (err) {
      fs.rmSync(full, { force: true });
      throw new Error(`XenoNet couldn't make ${full} yours alone, so it didn't write the token there (${String(err?.message ?? err).split("\n")[0]})`);
    }
    fs.writeFileSync(full, `${token}\n`);
    return full;
  }
  if (fs.existsSync(full)) fs.chmodSync(full, 0o600);
  fs.writeFileSync(full, `${token}\n`, { mode: 0o600 });
  fs.chmodSync(full, 0o600);
  return full;
}

// Folders of dependencies, builds and caches, whose ignored files are many
// and none of them yours: the look past .gitignore leaves them out. What's
// tracked in them, or untracked and not ignored, is looked in as before.
export const HEAVY_FOLDERS = ["node_modules", "bower_components", ".venv", "venv", ".tox", "__pycache__", ".gradle", "Pods", ".terraform", ".next", ".nuxt", ".turbo", ".cache", "target", "DerivedData"];
const TOKEN_ANYWHERE = new RegExp(TOKEN_GREP, "g");

// git grep -o -z's answer: a file's name (as it is, never quoted), a NUL,
// and a token found in it, a line each. A name may hold a newline; a token
// never does.
function grepMatches(stdout) {
  const out = [];
  for (let at = 0; at < stdout.length; ) {
    const nul = stdout.indexOf("\0", at);
    if (nul < 0) break;
    const end = stdout.indexOf("\n", nul);
    out.push([stdout.slice(at, nul), stdout.slice(nul + 1, end < 0 ? stdout.length : end)]);
    if (end < 0) break;
    at = end + 1;
  }
  return out;
}

// The files in these git working trees that hold an actor's token, for the
// doctor, by git grep, which answers with each token it finds and the file
// it's in: what's tracked, what's untracked but not ignored, and the
// ignored files too (a .env, say, where tokens are most often left),
// outside HEAVY_FOLDERS. Only a token XenoNet issued counts: `issued` is
// the SHA-256 of each token that works now (the engine's
// issuedTokenHashes), so a made-up one in a test fixture or a fuzz seed
// isn't taken for a leak. Without it (XenoNet couldn't read its tokens),
// every token counts, as before, and the look says it couldn't check them
// (`unchecked`). A name that holds a token itself is shown without it.
// `run(file, args, ms)` runs a command (the doctor's own); repos are
// looked in `together` at a time, and none is started after `budgetMs`.
// { checked, files, unchecked? }: how many repos were looked in, and the
// files, absolute.
export async function findTokenFiles(repos, { run, issued = null, budgetMs = 5000, eachMs = 3000, together = 4, now = Date.now } = {}) {
  const started = now();
  const queue = [...repos];
  const files = [];
  let checked = 0;
  const known = issued == null ? null : new Set(issued);
  // Whether a token counts, hashed once however often it turns up.
  const verdicts = new Map();
  const counts = (token) => {
    if (!known) return true;
    if (!verdicts.has(token)) verdicts.set(token, known.has(hashToken(token)));
    return verdicts.get(token);
  };
  // Optional locks off and no fsmonitor, so looking changes nothing in the
  // repo and starts no daemon.
  const grep = (repo, more, ms) => run("git", ["--no-optional-locks", "-C", repo, "-c", "core.fsmonitor=false", "grep", "-o", "-z", "-I", "--untracked", ...more], ms);
  // 0: found some; 1: none; a timeout or anything else: not looked in.
  const answered = (r) => !r.timedOut && (r.code === 0 || r.code === 1);
  const next = async () => {
    while (queue.length && now() - started < budgetMs) {
      const repo = queue.shift();
      // As git sees the repo, then past its .gitignore. (With --untracked,
      // git grep doesn't go into an ignored folder even for what's tracked
      // there, so the second look also finds that.) Both name files the
      // same way, so a file both find is one. The two share a repo's time.
      const t0 = now();
      const seen = await grep(repo, ["-E", "-e", TOKEN_GREP], eachMs);
      const left = eachMs - (now() - t0);
      const ignored = !answered(seen) ? seen : left <= 0 ? { timedOut: true } : await grep(repo, ["--no-exclude-standard", "-E", "-e", TOKEN_GREP, "--", ".", ...HEAVY_FOLDERS.map((d) => `:(exclude,glob)**/${d}/**`)], left);
      if (!answered(seen) || !answered(ignored)) continue;
      checked++;
      const names = new Set();
      for (const r of [seen, ignored]) if (r.code === 0) for (const [name, token] of grepMatches(r.stdout)) if (counts(token)) names.add(name);
      for (const f of names) files.push(path.join(repo, f).replace(TOKEN_ANYWHERE, (m) => `${m.slice(0, m.lastIndexOf("_") + 1)}…`));
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, together) }, next));
  return { checked, files: files.sort(), ...(known ? {} : { unchecked: true }) };
}

// The doctor's look for token files, kept for a day (the decision at merge
// point B): the server's doctor runs every half hour, and looks again only
// once the last look is a day old, or when you run the doctor yourself
// (`fresh`). `repos()`: the repos to look in, then; `issued()`: the hashes
// of the tokens that work then (the engine store's issuedTokenHashes), or
// null, or a throw, when they can't be read. Looks don't overlap: one
// asked for during another gets that one. { at, checked, repos, files,
// unchecked? }.
export const TOKEN_SCAN_MS = 24 * 3600e3;
export function createTokenScan({ repos, run, issued = null, now = Date.now, everyMs = TOKEN_SCAN_MS, ...opts }) {
  let last = null;
  let looking = null;
  return {
    async look({ fresh = false } = {}) {
      if (looking) return looking;
      if (!fresh && last && now() - last.at < everyMs) return last;
      looking = (async () => {
        const list = repos();
        let hashes = null;
        try {
          hashes = issued?.() ?? null;
        } catch {}
        const found = await findTokenFiles(list, { run, now, issued: hashes, ...opts });
        return (last = { at: now(), repos: list.length, ...found });
      })().finally(() => (looking = null));
      return looking;
    },
    last: () => last,
  };
}
