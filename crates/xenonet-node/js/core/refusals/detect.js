// core/refusals/detect.js in Rust (crates/xenonet-sources/src/refusals/detect.rs):
// a provider's refusal, as a transcript line shows it (docs/guide/
// refusals.md), read from Claude Code's and Codex's own lines the way they
// mark it, never guessed from what a model wrote.
//
// claudeSignal(raw) and codexSignal(raw) take a line as the file has it
// and return { provider, kind, source, category, explanation, text,
// requestId, messageId, uuid, parentUuid, model, fallbackModel, code, at }
// or null. A credit token the API sends with a fallback is never kept.
import { native } from "../../index.js";

export const claudeSignal = (raw) => native.refusalClaudeSignal(raw);

export const codexSignal = (raw) => native.refusalCodexSignal(raw);

// How the transcripts' mapper (core/events.js) reads a line's refusal: the
// same signal.
export const claudeRefusal = claudeSignal;

export const codexRefusal = codexSignal;

// What makes two sightings of a refusal the same one: the request's id
// when there is one, else where it was written.
export const keyOf = (sig, sessionId) => native.refusalKeyOf(sig, sessionId);
