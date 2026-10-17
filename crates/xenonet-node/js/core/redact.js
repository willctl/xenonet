// core/redact.js in Rust (crates/xenonet-state/src/redact.rs): what an
// export of the day may say. Projects show as star names from a keyed hash
// of each one's id, the same all day and new the next; the salt is kept in
// XenoNet's state folder.
import { native } from "../index.js";

export const dayKey = (now = Date.now()) => native.redactDayKey(Number(now));

export const starName = (id, { salt, day = dayKey() }) => native.starName(String(id), String(salt), String(day));

export const starNames = (ids, { salt, day = dayKey() }) => new Map(native.starNames([...ids], String(salt), String(day)));

export const posterSalt = (dir) => native.posterSalt(dir ? String(dir) : null);

export const archetypeOf = ({ totals, hours } = {}) => native.archetypeOf({ totals, hours });

export function constellationOf(day, { salt, now = Date.now(), names = false, nodes = [] } = {}) {
  return native.constellationOf(day, String(salt), Number(now), !!names, nodes);
}

export function labelsOf(nodes, day, { salt, now = Date.now() } = {}) {
  return native.labelsOf(nodes, day, String(salt), Number(now));
}
