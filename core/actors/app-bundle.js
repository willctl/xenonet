// An actor product's Mac app, as its observer reads it (Grok Bot's,
// Muse's): whether it's installed where apps go, and from its Info.plist
// the keys asked for and nothing else of the app's: its version
// (CFBundleShortVersionString), and for an app whose name isn't its
// maker's alone, its bundle id (CFBundleIdentifier), which says whose it
// is. Never its data, logs, preferences or tokens. The Info.plist is read
// again only when it changes.
import fs from "node:fs";
import { execFile } from "node:child_process";
import { tilde, pathStyle } from "../paths.js";

// A version as an app gives it: short, with no spaces. Anything else in
// its place isn't shown.
const VERSION = /^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$/;
// A bundle id, reverse DNS, as Apple allows it.
const BUNDLE_ID = /^[A-Za-z0-9][A-Za-z0-9.-]{0,154}$/;

// A key's string from an Info.plist written as XML, that key alone, or
// null.
export function plistString(text, key) {
  const m = new RegExp(`<key>\\s*${key}\\s*</key>\\s*<string>\\s*([^<]*?)\\s*</string>`).exec(String(text ?? ""));
  return m ? m[1] : null;
}

// A key from an Info.plist written as binary, which plutil reads out
// alone; null when it can't.
const binaryString = (file, key) =>
  new Promise((resolve) => execFile("plutil", ["-extract", key, "raw", "-o", "-", file], { timeout: 3000 }, (err, stdout) => resolve(err ? null : String(stdout).trim())));

const why = (err) => (err.code === "ENOENT" || err.code === "ENOTDIR" ? "missing" : err.code === "EACCES" || err.code === "EPERM" ? "denied" : "unreadable");

export class AppBundle {
  // `apps`: where the app may be installed. `id`: the bundle ids that are
  // the product's, for an app another maker's could share a name with;
  // null to take any app of its name, and never read its id. `files`: how
  // files are read. `binary(file, key)`: how a key is read from an
  // Info.plist written as binary.
  constructor({ apps, id = null, files = fs, binary = binaryString }) {
    this.apps = apps;
    this.id = id;
    this.files = files;
    this.binary = binary;
    // The last read, and the Info.plist it came from as it was then:
    // { file, stamp, version, id, state }.
    this.plist = null;
  }

  // Where the app is, or null.
  where() {
    return (
      this.apps.find((p) => {
        try {
          return this.files.existsSync(p);
        } catch {
          return false;
        }
      }) ?? null
    );
  }

  // Whether the app is installed: there, and the product's by its bundle id
  // as last read (read() says).
  installed() {
    return this.where() != null && this.ours();
  }

  // Whether the last read found the product's bundle id. An id that
  // couldn't be read isn't taken for the product's. Before any read (its
  // observer off, say), an app of its name counts, since telling whose it
  // is would mean reading it.
  ours() {
    if (!this.id || this.plist == null) return true;
    return this.plist.id != null && this.id.test(this.plist.id);
  }

  // The installed app's version, read with its bundle id when that's asked
  // for, again only when the file changes. Null when it can't be read.
  async version() {
    return (await this.read()).version;
  }

  async read() {
    const app = this.where();
    const file = app ? pathStyle(app).join(app, "Contents", "Info.plist") : null;
    let stamp = null;
    let failed = null;
    try {
      if (file) {
        const st = this.files.statSync(file);
        stamp = `${st.mtimeMs}:${st.size}`;
      }
    } catch (err) {
      [stamp, failed] = [err.code ?? "?", err];
    }
    if (this.plist && this.plist.file === file && this.plist.stamp === stamp) return this.plist;
    const read = { file, stamp, version: null, id: null, state: failed ? why(failed) : "missing" };
    this.plist = read;
    if (!file || failed) return read;
    try {
      const raw = this.files.readFileSync(file);
      const bytes = Buffer.isBuffer(raw) ? raw : Buffer.from(String(raw));
      const binary = bytes.subarray(0, 6).toString("latin1") === "bplist";
      const key = async (k) => (binary ? await this.binary(file, k) : plistString(bytes.toString("utf8"), k));
      const version = await key("CFBundleShortVersionString");
      read.version = version != null && VERSION.test(version) ? version : null;
      if (this.id) {
        const id = await key("CFBundleIdentifier");
        read.id = id != null && BUNDLE_ID.test(id) ? id : null;
      }
      read.state = read.version ? "ok" : "unreadable";
    } catch (err) {
      read.state = why(err);
    }
    return read;
  }

  // What the doctor shows of the app: whether it's there (and, when its id
  // is asked, whether it's the product's: "other" for another maker's app
  // of its name), and whether its version could be read.
  sources(home) {
    const app = this.where();
    const at = app ?? this.apps[0];
    return [
      { name: "app", file: tilde(at, home), state: !app ? "missing" : this.plist?.id != null && !this.ours() ? "other" : "ok" },
      { name: "app's version", file: tilde(this.plist?.file ?? pathStyle(at).join(at, "Contents", "Info.plist"), home), state: this.plist?.state ?? "missing" },
    ];
  }
}
