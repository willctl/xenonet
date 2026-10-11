// xenonet search <words>: every Claude Code, Codex and Grok Build transcript, through the
// running XenoNet's index (core/search.js).
import { c, print, json, plural, ago, Fail, width, clip } from "../lib/out.js";
import { need, get } from "../../core/client.js";

export const help = `Searches every Claude Code, Codex and Grok Build transcript on this machine for the
messages that have every word (or a word that starts with it), grouped by
session, best matches first.

  --limit <n>  how many sessions (default 10)
  --json       the results as JSON`;

export async function run(words, flags) {
  const q = words.join(" ").trim();
  if (!q) throw new Fail("say what to look for: xenonet search <words>", 2);
  const s = await need(flags);
  const { results, status } = await get(s, `/api/search?q=${encodeURIComponent(q)}`);
  const top = results.slice(0, Number(flags.limit) || 10);
  if (flags.json) return json({ results: top, status });
  if (!top.length) return print(c.dim(`Nothing in ${status?.docs ? plural(status.docs, "indexed message") : "the transcripts"} matches "${q}".`));
  const room = width() - 4;
  for (const r of top) {
    const when = r.snippets[0]?.ts ? ago(r.snippets[0].ts) : null;
    print(`${c.bold(r.title ?? r.sessionId)} ${c.dim([r.app, plural(r.hits, "hit"), when].filter(Boolean).join(" · "))}`);
    for (const sn of r.snippets.slice(0, 2)) {
      const text = clip(sn.text.replace(/\s+/g, " "), room - sn.role.length - 2);
      print(`  ${c.dim(`${sn.role}:`)} ${text.replace(/⟦([^⟧]*)⟧/g, (_, m) => c.yellow(m)).replace(/[⟦⟧]/g, "")}`);
    }
  }
  if (results.length > top.length) print(c.dim(`\n${results.length - top.length} more; --limit ${results.length} shows them.`));
}
