// Apps opened from Finder, the Dock or at login get launchd's bare PATH
// (/usr/bin:/bin:/usr/sbin:/sbin) and no LANG, so the codex CLI, Homebrew's
// git and anything set up in the shell profile are missing, and shells start
// without UTF-8. Ask the login shell for its PATH once, the way a new
// Terminal window would see it, and fill in LANG from the system locale.
// The server and the terminal's shells inherit both.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const MARK = "__XENONET_ENV__";

function shellPath(shell, timeout) {
  const fish = path.basename(shell) === "fish";
  const script = fish ? `printf '${MARK}%s${MARK}' (string join : $PATH)` : `printf '${MARK}%s${MARK}' "$PATH"`;
  return new Promise((resolve) => {
    // -i reads ~/.zshrc and friends, where most people extend PATH.
    execFile(shell, ["-ilc", script], { timeout, encoding: "utf8", env: { ...process.env, DISABLE_AUTO_UPDATE: "true" } }, (_err, stdout) => {
      resolve(new RegExp(`${MARK}(.*?)${MARK}`, "s").exec(stdout ?? "")?.[1] ?? null);
    });
  });
}

export function mergePath(...lists) {
  const seen = new Set();
  return lists
    .flatMap((l) => String(l ?? "").split(":"))
    .filter((d) => d && !seen.has(d) && seen.add(d))
    .join(":");
}

// en-US → en_US.UTF-8, if the system has that locale; else en_US.UTF-8, so
// shells always get UTF-8 rather than the C locale.
export function langFor(locale, exists = (name) => fs.existsSync(path.join("/usr/share/locale", name))) {
  const m = /^([a-z]{2,3})[-_]([A-Za-z]{2})\b/.exec(String(locale ?? ""));
  const lang = m && `${m[1]}_${m[2].toUpperCase()}.UTF-8`;
  return lang && exists(lang) ? lang : "en_US.UTF-8";
}

export async function loadShellEnv({ locale, timeout = 4000 } = {}) {
  if (process.platform === "win32") return;
  // Started from a terminal: the environment is already the shell's.
  if (!process.env.TERM_PROGRAM) {
    const shell = process.env.SHELL || os.userInfo().shell || "/bin/zsh";
    const fromShell = await shellPath(shell, timeout);
    const common = ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", path.join(os.homedir(), ".local", "bin")];
    process.env.PATH = mergePath(fromShell, process.env.PATH, common.join(":"));
  }
  if (process.platform === "darwin" && !process.env.LANG && !process.env.LC_ALL) process.env.LANG = langFor(locale);
}
