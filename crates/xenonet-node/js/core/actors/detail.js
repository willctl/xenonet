// core/actors/detail.js in Rust (crates/xenonet-actors/src/detail.rs): what
// actors' events say, written from facts, and a status line, checked.
import { native } from "../../index.js";

export const APPS = native.detailApps();
export const STATUS_MAX = native.detailStatusMax();

export const clip = (s, n) => (s == null ? null : native.detailClip(String(s), Number(n)));
export const sessionWords = (app, place) => native.detailSessionWords(app, place);
export const presenceWords = (from, to) => native.detailPresenceWords(from, to);
export const writeDetail = (kind, facts = {}, where = null) => native.detailWrite(kind, facts, where) ?? null;
export const consoleStatus = (what, where) => native.detailConsoleStatus(what, where) ?? null;
export const looksLikeKey = (s) => native.detailLooksLikeKey(String(s));
export const placeName = (name, kind) => (name == null ? null : native.detailPlaceName(String(name), kind));
export const statusLine = (text) => native.detailStatusLine(text);
