// core/assistant/credentials.js in Rust (crates/xenonet-assistant/src/
// credentials.rs): grant credentials and tombstones on disk. The checks
// that a credential stays private are private-file.js's, asked here; the
// files, their modes and the order they're written in are the Rust's.
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { errorOf } from "../../settle.js";

const { protectFile, otherReaders } = await import(peer("private-file.js"));

export const TOKEN_PREFIX = native.asstConstants().tokenPrefix;
export const TOKEN = /^xenonet_assistant_[0-9a-f]{64}$/;
export const isToken = (s) => native.asstIsToken(s);
export const hashToken = (token) => native.asstHashToken(String(token));

export const newToken = () => native.asstNewToken();

export const assistantDir = (stateDir) => native.asstAssistantDir(String(stateDir));

const settle = (r) => {
  if (r.error) throw errorOf(r.error);
  return r.ok;
};
// A callback the Rust asks, as { ok } or { error }.
const tried = (fn) => (file) => {
  try {
    return { ok: fn(file) ?? null };
  } catch (err) {
    return { error: String(err?.message ?? err) };
  }
};

export function writeCredential(dir, { grant, token, device, expiresAt = null }, { protect = protectFile, readers = otherReaders } = {}) {
  return settle(native.asstWriteCredential(String(dir), { grant, token, device, expiresAt }, tried(protect), tried(readers)));
}

export const readCredential = (dir, grant) => native.asstReadCredential(String(dir), grant);

export const deleteCredential = (dir, grant) => settle(native.asstDeleteCredential(String(dir), grant));

export const markRevision = (dir, grant, revision) => settle(native.asstMarkRevision(String(dir), grant, revision));
export const revisionOf = (dir, grant) => native.asstRevisionOf(String(dir), grant) ?? null;

export const markRevoked = (dir, grant, at = Date.now()) => settle(native.asstMarkRevoked(String(dir), grant, Number(at)));
export const isMarkedRevoked = (dir, grant) => native.asstIsMarkedRevoked(String(dir), grant);
export const markedRevoked = (dir) => native.asstMarkedRevoked(String(dir));

export const markDisabled = (dir, at = Date.now()) => settle(native.asstMarkDisabled(String(dir), Number(at)));
export const isDisabled = (dir) => native.asstIsDisabled(String(dir));
export const clearDisabled = (dir) => settle(native.asstClearDisabled(String(dir)));
