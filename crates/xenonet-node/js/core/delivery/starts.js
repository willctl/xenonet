// core/delivery/starts.js in Rust (crates/xenonet-engine/src/delivery/starts.rs):
// what a start request may say, how its first prompt is framed, the
// command that starts it, the limits and what the answers say.
import { native } from "../../index.js";

const C = native.startsConstants();
export const TARGET = C.target;
export const APPS = C.apps;
export const MODES = C.modes;
export const PROMPT_MAX = C.promptMax;
export const PER_HOUR = C.perHour;
export const PER_HOST_HOUR = C.perHostHour;
export const FRESH_MS = C.freshMs;

export const request = (v = {}) => native.startsRequest(v ?? {});

export const framed = (by, from, prompt) => native.startsFramed(by ?? {}, from ?? null, String(prompt ?? ""));

export const command = (r, id, cwd, { settings = null, server = "xenonet", promptFile = null } = {}) => native.startsCommand(r, String(id), String(cwd), settings, String(server), promptFile);

export const admit = (history, from, now = Date.now()) => native.startsAdmit(history, String(from ?? ""), Number(now)) ?? null;

export const kept = (history, now = Date.now()) => native.startsKept(history, Number(now));

export const folder = (asked, home, known = [], reported = []) => native.startsFolder(String(asked ?? ""), String(home), known, reported.map(String));

export const signedOut = (log) => native.startsSignedOut(String(log ?? ""));

export const sessionOf = (app, log) => native.startsSessionOf(String(app), String(log ?? "")) ?? null;

export const answerWords = (state, host, a = {}) => native.startsAnswerWords(String(state), String(host), a ?? {});

export const signInWords = (host) => native.startsSignInWords(String(host));

export const SIGN_IN_TARGET = C.signInTarget;

export const signInToken = (pasted) => native.startsSignInToken(String(pasted ?? ""));
