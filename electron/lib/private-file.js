// Private files use Unix modes or Windows ACLs, never Windows mode bits.
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// Endpoint inspection can delay even whoami on a busy Windows workstation.
// Keep a bounded wait and propagate failures before touching private files.
export function runPrivateCommand(file, args, { exec = execFileSync } = {}) {
  return exec(file, args, { encoding: "utf8", windowsHide: true, timeout: 60000 });
}
const runSync = runPrivateCommand;

// Your account's SID, from whoami /user: the last field of its CSV line.
export function userSid({ run = runSync } = {}) {
  const sid = /"(S-1-[0-9-]+)"\s*$/m.exec(String(run("whoami", ["/user", "/fo", "csv", "/nh"])))?.[1];
  if (!sid) throw new Error("whoami didn't say which account this is");
  return sid;
}

// A file's ACL as SDDL, which names accounts by SID in any language:
// icacls /save writes it to a file of its own (in UTF-16, after a line
// with the file's name), which is read and taken away.
export function aclOf(file, { run = runSync } = {}) {
  const out = path.join(os.tmpdir(), `xenonet-acl-${process.pid}-${crypto.randomBytes(6).toString("hex")}.txt`);
  try {
    run("icacls", [file, "/save", out, "/q"]);
    const raw = fs.readFileSync(out);
    const text = raw[0] === 0xff && raw[1] === 0xfe ? raw.subarray(2).toString("utf16le") : raw.length > 1 && raw[1] === 0 ? raw.toString("utf16le") : raw.toString("utf8");
    const line = text.split(/\r?\n/).find((l) => /D:[A-Z]*(\(|NO_ACCESS_CONTROL)/.test(l));
    if (!line) throw new Error(`icacls gave no ACL for ${file}`);
    return line.slice(line.indexOf("D:"));
  } finally {
    fs.rmSync(out, { force: true });
  }
}

// The accounts an ACL (SDDL) lets read a file besides yours (`sid`),
// SYSTEM and the Administrators, which OpenSSH for Windows allows on a key
// too: those of allow entries that apply to the file itself, and everyone
// when there's no ACL at all.
// icacls can shorten the built-in Administrator account to LA, including
// when that account was renamed. It has the administrator access allowed
// above; treating it as an unknown user refuses files we just protected.
const TRUSTED = new Set(["SY", "S-1-5-18", "BA", "S-1-5-32-544", "LA", "OW", "S-1-3-4"]);
export function othersIn(sddl, sid) {
  if (/D:[A-Z]*NO_ACCESS_CONTROL/.test(sddl)) return ["everyone"];
  const others = [];
  for (const [, ace] of String(sddl).matchAll(/\(([^)]*)\)/g)) {
    const [type, flags = "", , , , trustee = ""] = ace.split(";");
    if (!["A", "OA", "XA", "ZA"].includes(type) || /IO/.test(flags) || trustee === sid || TRUSTED.has(trustee)) continue;
    others.push(trustee);
  }
  return others;
}

export function otherReaders(file, { platform = process.platform, run = runSync } = {}) {
  if (platform === "win32") return othersIn(aclOf(file, { run }), userSid({ run }));
  return fs.statSync(file).mode & 0o077 ? ["others"] : [];
}

export function protectFile(file, { platform = process.platform, run = runSync } = {}) {
  if (platform === "win32") run("icacls", [file, "/inheritance:r", "/grant:r", `*${userSid({ run })}:F`, "/q"]);
  else fs.chmodSync(file, 0o600);
}
