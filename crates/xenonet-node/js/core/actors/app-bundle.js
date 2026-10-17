// core/actors/app-bundle.js in Rust (crates/xenonet-actors/src/app_bundle.rs):
// an actor product's Mac app, as its observer reads it. What was read and
// what it means are Rust's; the files are read through `files`, and a key
// of a binary Info.plist through `binary` (plutil), here.
import fs from "node:fs";
import { execFile } from "node:child_process";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";

export const plistString = (text, key) => native.plistString(String(text ?? ""), String(key)) ?? null;

const binaryString = (file, key) =>
  new Promise((resolve) => execFile("plutil", ["-extract", key, "raw", "-o", "-", file], { timeout: 3000 }, (err, stdout) => resolve(err ? null : String(stdout).trim())));

const CORE = Symbol("xenonet.app-bundle");

export class AppBundle {
  constructor({ apps, id = null, files = fs, binary = binaryString }) {
    this.apps = apps;
    this.id = id;
    this.files = files;
    this.binary = binary;
    this[CORE] = new native.AppBundleCore();
  }

  // The last read, and the Info.plist it came from as it was then.
  get plist() {
    return this[CORE].plist();
  }

  where() {
    return this[CORE].where(this) ?? null;
  }
  installed() {
    return this[CORE].installed(this);
  }
  ours() {
    return this[CORE].ours(this);
  }
  async version() {
    return (await this.read()).version;
  }
  async read() {
    return drive(this[CORE].read(), calls(this));
  }
  sources(home) {
    return this[CORE].sources(this, home);
  }
}
