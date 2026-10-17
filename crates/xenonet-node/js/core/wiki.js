// core/wiki.js in Rust (crates/xenonet-sources/src/wiki.rs): the shared
// context's folder as a constellation: pages, their [[links]], the projects
// they mention, recent writes from the activity log, and notes still waiting
// in the inbox for the curator. Rust reads the folder too.
import { native } from "../index.js";

// The text after the frontmatter is sliced off what was given, so a text
// without any comes back as it was.
export function parseFrontmatter(text) {
  const { meta, end } = native.wikiParseFrontmatter(String(text));
  return { meta, body: end === null ? text : text.slice(end) };
}

export const wikiLinks = (text) => native.wikiLinks(String(text));

export const mentionedProjects = (text, tags, projects, root = null) => native.wikiMentionedProjects(String(text), tags, projects, root);

export const parseLog = (text, date) => native.wikiParseLog(String(text), date);

// The time is the caller's, or Date.now(), which tests mock.
export async function readWiki(dir, projects, { root = null, now = Date.now(), server = "agent-wiki" } = {}) {
  return native.readWiki(dir, projects, root, Number(now), server);
}
