// Grant credentials and tombstones, on disk (docs/design/cloud-assistants.md,
// section 4). All of it in the instance's state folder, under assistant/:
//
//   credentials/<grant>.json  a grant's credential, for the adapter that
//                             grant's launch profile starts (xenonet mcp
//                             assistant --grant <grant>): written once, at
//                             your approval, readable only by your account
//                             (private-file.js), and deleted when the grant
//                             is revoked. XenoNet keeps only its SHA-256
//   revoked/<grant>           a grant revoked, whatever engine.sqlite says:
//                             a database restored from a copy, or another
//                             version of XenoNet, can't bring it back
//   revisions/<grant>         a grant's revision now, so an adapter with no
//                             service to ask never serves what it read
//                             under a scope that has since changed
//   disabled                  every cloud connection off, until you turn
//                             them on again in XenoNet's window. Written by
//                             the service, or by xenonet assistant disable
//                             without one
//
// A credential never goes in an MCP tool's arguments, an environment
// variable or a log; the adapter reads it here and sends it only over
// XenoNet's private local connection.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { protectFile, otherReaders } from "../private-file.js";
import { GRANT_ID } from "../schema/assistant.js";

export const TOKEN_PREFIX = "xenonet_assistant_";
export const TOKEN = /^xenonet_assistant_[0-9a-f]{64}$/;
export const isToken = (s) => typeof s === "string" && TOKEN.test(s);
export const hashToken = (token) => crypto.createHash("sha256").update(String(token)).digest("hex");

// A new credential, and the hash XenoNet keeps of it: 256 random bits.
export function newToken() {
  const token = TOKEN_PREFIX + crypto.randomBytes(32).toString("hex");
  return { token, hash: hashToken(token) };
}

export const assistantDir = (stateDir) => path.join(stateDir, "assistant");
const credentialFile = (dir, grant) => path.join(dir, "credentials", `${grant}.json`);
const checkGrant = (grant) => {
  if (!GRANT_ID.test(String(grant))) throw new Error("not a grant id");
  return grant;
};

// Writes a grant's credential where only your account can read it, and
// checks that it can't be read by anyone else; a file that can is deleted
// and the write fails. `protect` and `readers` stand in for tests.
export function writeCredential(dir, { grant, token, device, expiresAt = null }, { protect = protectFile, readers = otherReaders } = {}) {
  const file = credentialFile(dir, checkGrant(grant));
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  fs.rmSync(file, { force: true });
  try {
    fs.writeFileSync(file, "", { flag: "wx", mode: 0o600 });
    protect(file);
    fs.writeFileSync(file, `${JSON.stringify({ schema: "assistant/credential/v1", grant_id: grant, device_id: device, expires_at: expiresAt, token })}\n`);
    const others = readers(file);
    if (others.length) throw new Error(`others could read it (${others.slice(0, 3).join(", ")})`);
  } catch (err) {
    fs.rmSync(file, { force: true });
    throw new Error(`XenoNet couldn't keep the grant's credential private: ${err.message}`);
  }
  return file;
}

// A grant's credential, with when its grant expires, or null when there's
// none (revoked, or never written on this computer).
export function readCredential(dir, grant) {
  try {
    const c = JSON.parse(fs.readFileSync(credentialFile(dir, checkGrant(grant)), "utf8"));
    return c?.grant_id === grant && isToken(c.token) ? { token: c.token, device: c.device_id ?? null, expiresAt: Number.isFinite(c.expires_at) ? c.expires_at : null } : null;
  } catch {
    return null;
  }
}

export function deleteCredential(dir, grant) {
  fs.rmSync(credentialFile(dir, checkGrant(grant)), { force: true });
}

// A grant's revision, as the service last moved it on, or null when there's
// no record of it (an adapter then serves nothing from its cache).
export function markRevision(dir, grant, revision) {
  const file = path.join(dir, "revisions", checkGrant(grant));
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  fs.writeFileSync(file, `${revision}\n`, { mode: 0o600 });
}
export function revisionOf(dir, grant) {
  try {
    const n = Number(fs.readFileSync(path.join(dir, "revisions", checkGrant(grant)), "utf8").trim());
    return Number.isInteger(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
}

// ---------- tombstones ----------

export function markRevoked(dir, grant, at = Date.now()) {
  const file = path.join(dir, "revoked", checkGrant(grant));
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  try {
    fs.writeFileSync(file, `${JSON.stringify({ at })}\n`, { flag: "wx", mode: 0o600 });
  } catch (err) {
    if (err.code !== "EEXIST") throw err;
  }
}
export const isMarkedRevoked = (dir, grant) => GRANT_ID.test(String(grant)) && fs.existsSync(path.join(dir, "revoked", grant));
export function markedRevoked(dir) {
  try {
    return fs.readdirSync(path.join(dir, "revoked")).filter((g) => GRANT_ID.test(g));
  } catch {
    return [];
  }
}

export function markDisabled(dir, at = Date.now()) {
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(dir, "disabled"), `${JSON.stringify({ at })}\n`, { mode: 0o600 });
}
export const isDisabled = (dir) => fs.existsSync(path.join(dir, "disabled"));
export function clearDisabled(dir) {
  fs.rmSync(path.join(dir, "disabled"), { force: true });
}
