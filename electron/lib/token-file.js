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
import { aclOf, userSid, othersIn, protectFile } from "./private-file.js";
export { aclOf, userSid, othersIn } from "./private-file.js";
const runSync = (file, args) => execFileSync(file, args, { encoding: "utf8", windowsHide: true, timeout: 10000 });
import { isToken } from "./tokens.js";
import { pathKey, isUnder } from "./paths.js";

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
