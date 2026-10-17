// core/classify.js in Rust (crates/xenonet-state/src/classify.rs): the
// optional pass over the projects folder by Codex or a decision model.
// Rust makes the outline, the prompt and the questions, and reads the
// answers; running Codex (digest.js's askCodex) and asking the decider stay
// here.
import { native } from "../index.js";
import { askCodex } from "./digest.js";

export const PICKS_SCHEMA = native.picksSchema();

export const outlineFolders = (folders) => native.outlineFolders(folders);
export const fingerprint = (outline) => native.fingerprint(outline);
export const picksPrompt = (outline) => native.picksPrompt(outline);
export const parsePicks = (text, outline) => native.parsePicks(String(text), outline);

export async function askCodexForPicks(codexBin, outline, opts) {
  return parsePicks(await askCodex(codexBin, picksPrompt(outline), { schema: PICKS_SCHEMA, timeoutMs: 180000, ...opts }), outline);
}

export const CONFIDENT = native.confident();

export const jevQuestions = (outline) => native.jevQuestions(outline);
export const picksFromAnswers = (answers, folders, offset = 0) => native.picksFromAnswers(answers, folders, Number(offset));

export async function askDeciderForPicks(decider, outline) {
  const state = { about: native.deciderAbout(), folders: outline };
  const picks = [];
  let offset = 0;
  for (const batch of jevQuestions(outline)) {
    const r = await decider.decide({ kind: "projects", state, questions: batch.questions });
    picks.push(...picksFromAnswers(r.answers, batch.folders, offset));
    offset += batch.folders.length;
  }
  return picks;
}
