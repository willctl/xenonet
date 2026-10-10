// xenonet relay: this XenoNet's map, streamed to a console on another machine
// over the SSH connection the console opened (core/hosts/relay.js).
// Consoles run it, as ssh <host> xenonet relay; nothing listens anywhere.
import { runRelay, parseSince, fromOriginalCommand, DEFAULT_SINCE_MS } from "../../core/hosts/relay.js";
import { candidates } from "../../core/client.js";
import { Fail } from "../lib/out.js";

export const help = `Streams this machine's XenoNet to a console on another machine, read-only,
as one JSON object per line on standard output. A console with hosts on
runs it over SSH (ssh <host> xenonet relay); you don't need to run it
yourself. It reads the XenoNet on 127.0.0.1 and never opens a port.

  --port <n>     the XenoNet to read (47840, or 47841 with --demo)
  --since <age>  sessions updated this recently: 7d (the default), 12h, 90m
  --detail       live detail too (questions, last messages, tool details,
                 files touched), not only titles, states and counts
  --start        start a XenoNet here if none is running, for as long as the
                 relay runs

A key restricted to the relay in authorized_keys
(command="xenonet relay --port 47840") may still ask for --since.`;

export async function run(_args, flags) {
  const forced = process.env.SSH_ORIGINAL_COMMAND ? fromOriginalCommand(process.env.SSH_ORIGINAL_COMMAND) : {};
  const sinceMs = flags.since !== undefined ? parseSince(flags.since) : (forced.sinceMs ?? DEFAULT_SINCE_MS);
  if (!sinceMs) throw new Fail(`--since is a number and m, h or d, such as 7d, 12h or 90m, not ${flags.since}`, 2);
  const port = Number(candidates(flags)[0]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Fail(`--port is a number from 1 to 65535`, 2);
  const code = await runRelay({ port, sinceMs, detail: flags.detail === true, start: flags.start === true, demo: flags.demo === true });
  // Whatever is still on its way to the console goes before the exit.
  await new Promise((resolve) => process.stdout.write("", resolve));
  process.exit(code);
}
