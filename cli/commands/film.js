// xenonet film: a film of the day (src/film.js), rendered in a hidden XenoNet
// Dev window and recorded to an MP4 (electron/recorder.js), through the
// control channel (core/control.js). Needs a checkout of XenoNet and ffmpeg.
import os from "node:os";
import { c, print, json, Fail } from "../lib/out.js";
import { xenonet } from "../../core/control.js";
import { find, get } from "../../core/client.js";
import { tilde } from "../../core/paths.js";
import { FILM_FRAMES, FILM_SECONDS, filmSeconds, filmPlaySeconds } from "../../core/frames.js";

export const help = `Makes a film of the last 24 hours: a title card, the day played back in
a slowly turning map with running counts, and an end card with the day's
numbers. It renders in a hidden window, so you can keep working, and
saves an MP4 to your videos folder. Projects show as star names, and no
clock appears, unless you reveal more. Nothing is uploaded, and the file
carries no metadata.

  --seconds <n>     how long the film is, cards included (${FILM_SECONDS.min} to ${FILM_SECONDS.max}; default ${FILM_SECONDS.default})
  --names           the projects' names, in place of the star names
  --tokens          the tokens processed, with the day's numbers
  --theme <name>    the look (default: the one XenoNet Dev uses)
  --size <WxH>      the frame (default ${FILM_FRAMES.story.width}x${FILM_FRAMES.story.height}, a story; wide is ${FILM_FRAMES.wide.width}x${FILM_FRAMES.wide.height})
  --out <file>      where to save it
  --instance demo   the demo's made-up day instead of yours

It needs ffmpeg (brew install ffmpeg), and runs from a checkout of XenoNet.`;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function run(_args, flags) {
  const instance = flags.instance || "film";
  const total = filmSeconds(flags.seconds);
  const seconds = filmPlaySeconds(total);
  const reveal = { names: !!flags.names, tokens: !!flags.tokens };
  const [w, h] = String(flags.size ?? `${FILM_FRAMES.story.width}x${FILM_FRAMES.story.height}`).split("x").map(Number);
  if (!(w >= 320 && h >= 240)) throw new Fail("--size is WIDTHxHEIGHT, such as 1080x1920", 2);
  const s = xenonet(instance);
  print(c.dim(`Starting ${s.name}, hidden…`));
  await s.launch({ show: "hidden" });

  // The look: the one asked for, or XenoNet Dev's.
  const dev = await find({ dev: true });
  const theme = flags.theme ?? (dev ? (await get(dev, "/api/theme").catch(() => null))?.id : null);
  if (theme) {
    const here = { base: `http://127.0.0.1:${s.port}`, port: s.port };
    // Drawn in place, without a reload: the driver returns once it's drawn.
    if ((await get(here, "/api/theme")).id !== theme) await s.driver("theme", theme);
  }

  const started = await s.record({ action: "start", size: [w, h], width: w, ...(flags.out ? { file: flags.out } : {}) });
  print(c.dim(`Recording ${total} seconds…`));
  let result;
  try {
    await s.driver("film", { seconds, reveal });
    // Until the end card has come and gone.
    const deadline = Date.now() + (total + 40) * 1000;
    let seen = false;
    for (;;) {
      const st = await s.driver("state");
      if (st.overlays.film) seen = true;
      else if (seen) break;
      if (Date.now() > deadline) throw new Error("the film didn't finish in time");
      await sleep(500);
    }
  } finally {
    result = await s.record({ action: "stop" }).catch((err) => ({ error: err.message }));
    if (!flags.keep) await s.quit().catch(() => {});
  }
  if (result.error) throw new Fail(`the film didn't save: ${result.error}`);
  if (flags.json) return json({ ...result, theme: theme ?? null, instance, reveal });
  print(`${c.green("✓")} ${tilde(result.file, os.homedir())} ${c.dim(`(${result.seconds} s, ${(result.bytes / 1e6).toFixed(1)} MB${started.width ? `, ${started.width} wide` : ""})`)}`);
}
