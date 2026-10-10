// xenonet decide: the decision model XenoNet can ask instead of Codex
// (core/decide.js), and its OpenRouter key, which goes straight into your
// computer's credential store. The key is read from standard input, so it
// never shows in your shell's history or a process list.
import { c, print } from "../lib/out.js";
import { createSecretStore } from "../../core/secret-store.js";
import { createDecider } from "../../core/decide.js";

export const help = `The decision model XenoNet can ask some of what it now asks Codex, such as
which folders are projects, through OpenRouter, and its key.

  xenonet decide            which model, and whether a key is kept
  xenonet decide key        keep an OpenRouter key: paste it, then Enter (or
                          pipe it in); it goes to your credential store
  xenonet decide forget     forget the key

XENONET_DECISION_MODEL picks another model (the default is Jev,
~typesafe/jev-latest). Give the key a credit limit on OpenRouter.`;

async function readKey() {
  if (process.stdin.isTTY) process.stdout.write("Paste the OpenRouter key, then Enter: ");
  let text = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    text += chunk;
    if (process.stdin.isTTY && text.includes("\n")) break;
  }
  if (process.stdin.isTTY) process.stdout.write("\n");
  return text.trim();
}

export async function run([sub = "status"]) {
  const decider = createDecider({ secrets: createSecretStore() });
  if (sub === "status") {
    const s = await decider.status();
    return print(`${s.model} · ${s.key === "stored" ? c.green(`a key is kept in ${s.store}`) : "no key yet: xenonet decide key"}`);
  }
  if (sub === "key") {
    const r = await decider.storeKey(await readKey());
    return print(`${c.green("✓")} The key is kept in ${r.store}. XenoNet uses it only when you send something to the decision model.`);
  }
  if (sub === "forget") {
    await decider.forgetKey();
    return print(`${c.green("✓")} XenoNet forgot the key.`);
  }
  throw new Error(`no such: xenonet decide ${sub}. xenonet decide --help says more.`);
}
