// A product video made from XenoNet's synthetic workspace and real control channel.
// Run through `xenonet script run`; see docs/guide/showcase.md.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { findFfmpeg } from "../core/dirs.js";
import { prepareAudio, startAudio, stopAudio } from "./showcase-audio.mjs";

// A clock time or a date in the page's text, which the video must not show:
// a clock tells where it was recorded, and a date when.
const CLOCK = String.raw`\b\d{1,2}:\d{2}(?::\d{2})?\b|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]* \d{1,2}\b|\b(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)[a-z]*, |\b\d{1,2}/\d{1,2}/\d{2,4}\b|\b\d{4}-\d{2}-\d{2}\b|\bGMT\b|\bUTC\b`;

// Run in the page for the recording: the console's clock holds at its
// placeholder, and every date and clock time in the page's text becomes
// words that say nothing of when ("today", a session's span in minutes),
// now and as the page changes. Only what the page shows changes, not
// XenoNet's data. A watcher keeps any clock or date that shows anyway, for
// the script to refuse.
const SCRUB = String.raw`(() => {
  if (window.__showcaseScrub) return true;
  window.__xenonet.hud?.setClock?.("--:--:--");
  const MON = "(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*";
  const DAY = "(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)[a-z]*";
  const T = "(\\d{1,2}):(\\d{2})(?::\\d{2})? ?([AaPp][Mm])?";
  const minutes = (h, m, ap) => ((Number(h) % 12) + (ap && /p/i.test(ap) ? 12 : 0)) * 60 + Number(m);
  const nb = "\u00a0";
  const span = (a, b) => { const d = (b - a + 1440) % 1440; return d ? d + nb + "min" : "a" + nb + "moment"; };
  const rules = [
    [new RegExp(MON + " \\d{1,2},?(?: ·)? " + T + " to " + T, "gi"), (_, h1, m1, a1, h2, m2, a2) => "today" + nb + "·" + nb + span(minutes(h1, m1, a1 || a2), minutes(h2, m2, a2))],
    [new RegExp(T + " to " + T, "gi"), (_, h1, m1, a1, h2, m2, a2) => span(minutes(h1, m1, a1 || a2), minutes(h2, m2, a2))],
    [new RegExp(MON + " \\d{1,2},?(?: ·)? " + T, "gi"), () => "today"],
    [new RegExp("\\b" + DAY + ", " + MON + " \\d{1,2}\\b", "gi"), () => "today"],
    [new RegExp("\\b" + DAY + " " + T, "g"), () => "this week"],
    [new RegExp("\\b" + MON + " \\d{1,2}\\b", "g"), () => "today"],
    [/\b\d{1,2}\/\d{1,2}\/\d{2,4}\b|\b\d{4}-\d{2}-\d{2}\b/g, () => "today"],
    [new RegExp(" ?\\b" + T + "\\b", "g"), () => ""],
  ];
  const scrub = (node) => {
    let text = node.nodeValue;
    for (const [re, to] of rules) text = text.replace(re, to);
    if (text !== node.nodeValue) node.nodeValue = text;
  };
  const all = (root) => {
    if (root.nodeType === 3) return scrub(root);
    const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) scrub(n);
  };
  all(document.body);
  new MutationObserver((changes) => {
    for (const c of changes) {
      if (c.type === "characterData") scrub(c.target);
      else for (const n of c.addedNodes) all(n);
    }
  }).observe(document.body, { subtree: true, childList: true, characterData: true });
  const when = new RegExp(${JSON.stringify(CLOCK)}, "i");
  window.__showcaseClocks = new Set();
  setInterval(() => {
    const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      if (!when.test(n.nodeValue)) continue;
      const el = n.parentElement;
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height || r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
      if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
      window.__showcaseClocks.add(n.nodeValue.trim().slice(0, 80));
    }
  }, 250);
  window.__showcaseScrub = true;
  return true;
})()`;

export default async function ({ xenonet: s, step }) {
  if (s.id !== "showcase") throw new Error("Use --instance showcase so recording cannot include personal sessions.");
  const ffmpeg = findFfmpeg();
  if (!ffmpeg) throw new Error("Put ffmpeg on PATH before starting the recording instance.");
  // The page's clock runs in UTC, checked, and shows no time or date (SCRUB).
  await s.call({ op: "cdp", method: "Emulation.setTimezoneOverride", params: { timezoneId: "UTC" } });
  const zone = await s.eval("return { zone: Intl.DateTimeFormat().resolvedOptions().timeZone, offset: new Date().getTimezoneOffset() }");
  if (zone.zone !== "UTC" || zone.offset !== 0) throw new Error(`The page's clock isn't UTC: ${JSON.stringify(zone)}`);
  const root = path.resolve(import.meta.dirname, "..");
  const work = path.join(root, ".xenonet", "showcase");
  fs.mkdirSync(work, { recursive: true });
  const mark = fs.readFileSync(path.join(root, "public", "brand", "xenonet.svg"), "utf8").match(/<path d="([^"]+)"/)[1];
  const clips = [];
  const evidence = [];
  const appearance = [];
  // FFmpeg runs alongside, not blocking: a blocked script lets the app's
  // server close the idle connection the next control request would reuse.
  const encode = (...args) => new Promise((resolve, reject) => {
    const p = spawn(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", ...args], { stdio: "inherit" });
    p.on("error", reject);
    p.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`ffmpeg exited with ${code}`))));
  });

  // Title cards are drawn in code. Product footage is never reconstructed.
  async function artwork(name, title, detail, card = false, chapter = "") {
    const file = path.join(work, `${name}.png`);
    const png = await s.eval(`return (() => {
      const c = document.createElement('canvas'); c.width=1920; c.height=1080;
      const g=c.getContext('2d'); const card=${card};
      if(card){g.fillStyle='#05070A';g.fillRect(0,0,1920,1080);}
      g.fillStyle='#39FF9F'; g.save(); g.translate(card?160:48,card?156:30); g.scale(card?1.5:.8,card?1.5:.8); g.fill(new Path2D(${JSON.stringify(mark)})); g.restore();
      g.fillStyle='#DCE6EE';
      g.font=card?'500 36px "Segoe UI",sans-serif':'500 25px "Segoe UI",sans-serif';
      g.fillText('XenoNet',card?278:112,card?222:66);
      g.font=card?'500 76px "Segoe UI",sans-serif':'500 31px "Segoe UI",sans-serif';
      const lines=${JSON.stringify(title)}.split('\\n'); lines.forEach((line,i)=>g.fillText(line,card?160:400,(card?460:65)+i*96));
      g.fillStyle='#9FB0BD';g.font=card?'400 28px "Segoe UI",sans-serif':'400 22px "Segoe UI",sans-serif';
      g.fillText(${JSON.stringify(detail)},160,card?740:1050);
      g.fillStyle='#5F6F7C';g.font='400 18px "Segoe UI",sans-serif';
      if(!card){g.textAlign='right';g.fillText(${JSON.stringify(chapter)},1760,1050);}
      return c.toDataURL('image/png').split(',')[1];
    })()`);
    fs.writeFileSync(file, Buffer.from(png, "base64"));
    return file;
  }
  async function titleCard(name, title, detail, seconds) {
    const png = await artwork(name, title, detail, true);
    const file = path.join(work, `${name}.mp4`);
    const audio = path.join(work, `${name}-audio.webm`);
    await startAudio(s);
    await step(`sleep:${seconds * 1000}`);
    await stopAudio(s, audio);
    await encode("-loop", "1", "-i", png, "-i", audio, "-t", String(seconds), "-r", "30", "-vf", "fade=t=in:st=0:d=0.35,fade=t=out:st=" + (seconds-.35) + ":d=0.35,format=yuv420p", "-af", `apad,afade=t=in:d=0.15,afade=t=out:st=${seconds-.35}:d=0.35`, "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-flags:a", "+bitexact", "-c:v", "libx264", "-preset", "medium", "-crf", "19", file);
    clips.push(file);
  }
  async function clip(name, title, detail, action, seconds = 6, crop = null) {
    console.log(`Recording ${name}`);
    const raw = path.join(work, `${name}-raw.mp4`);
    const file = path.join(work, `${name}.mp4`);
    const audio = path.join(work, `${name}-audio.webm`);
    if (!(await s.eval("return window.__showcaseScrub === true"))) throw new Error("The page reloaded, and its clock and dates show again; record afresh.");
    const audioStartedAt = await startAudio(s);
    // keep: the screenshot after the clip is of the film's frame.
    const video = await s.record({ action: "start", file: raw, fps: 30, width: 1600, size: film, keep: true });
    let captured;
    try {
      await action();
      // This is editorial reading time, not a wait for the app to finish an action.
      await step(`sleep:${seconds * 1000}`);
    } finally {
      try { captured = await s.record({ action: "stop" }); }
      finally { await stopAudio(s, audio); }
    }
    const shot = await s.screenshot({ name: `showcase-${name}`, inline: false });
    const overlay = await artwork(name, title, detail, false, String(evidence.length + 1).padStart(2, "0"));
    const framing = crop ? `crop=${crop.join(":")},scale=1600:900:flags=lanczos,` : "";
    const audioOffset = (video.startedAt - audioStartedAt) / 1000;
    if (!Number.isFinite(audioOffset) || audioOffset < 0) throw new Error("Missing audio/video timing metadata.");
    const duration = captured.frames / 30;
    await encode("-i", raw, "-i", overlay, "-ss", String(audioOffset), "-i", audio, "-filter_complex", `[0:v]${framing}pad=1920:1080:160:112:color=0x05070A[frame];[frame][1:v]overlay=0:0:format=auto,format=yuv420p[out]`, "-map", "[out]", "-map", "2:a", "-af", `apad,afade=t=in:d=0.025,afade=t=out:st=${duration-.025}:d=0.025`, "-t", String(duration), "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-flags:a", "+bitexact", "-c:v", "libx264", "-preset", "medium", "-crf", "21", "-movflags", "+faststart", file);
    clips.push(file);
    evidence.push({ name, title, detail, ...captured, audio, audioOffset, screenshot: shot.file });
  }

  await prepareAudio(s);
  // The page at its own density, in a window the recorder makes 1600 by 900
  // points: on a Retina screen that's 3200 by 1800 pixels, scaled to the
  // film. A page size emulated inside a smaller window records zoomed in.
  const { viewport } = await s.window({ size: null });
  const film = [1600 * viewport.dpr, 900 * viewport.dpr];
  await s.driver("themeGallery", false);
  await s.driver("revertEnvironment");
  await step("theme:xenonet");
  await step("environment:nebula");
  await step("mask:on");
  await step("reset");
  await step("deselect");
  await s.eval(`return ${SCRUB}`);
  await titleCard("intro", "See what your\nsessions are doing.", "XenoNet maps every Claude Code and Codex session on your machine.", 3.5);
  await clip("overview", "See each session in its project.", "Claude Code and Codex sessions show up, with always-on agents beside them.", async () => {}, 5);
  const waiting = (await s.driver("state")).alerts.find(x => x.kind === "need");
  if (!waiting) throw new Error("The demo needs a session waiting for a question.");
  await clip("question", "Find the question that needs you.", "Read the session's context before you switch apps.", async () => {
    await s.driver("select", waiting.session);
    await s.input({ scroll: { x: 1460, y: 750, dy: -10000 } });
    await s.input({ scroll: { x: 1460, y: 750, dy: 310 } });
  }, 6, [1104, 622, 496, 278]);
  await step("deselect");
  await clip("edits", "Compare overlapping edits.", "XenoNet shows both changes and their branches.", async () => {
    const p = await s.eval(`return (() => { const r=document.querySelector('[data-al-collision]').getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}; })()`);
    await s.input({ click: p });
  }, 7, [1280, 720, 160, 90]);
  await s.input({ keys: "Escape" });
  await clip("agents", "See what an always-on agent is doing.", "Agents can find peers, claim files and send messages through XenoNet.", () => s.driver("select", "actor:wren"), 6);
  await step("deselect");
  await clip("control", "Agents can operate the console too.", "This recording uses the same controls for finding and selecting work.", async () => {
    const { nodes } = await s.driver("find", { query: "atlas-api", kind: "project", limit: 1 });
    await s.driver("select", nodes[0].id);
  }, 5);
  await step("deselect");
  await step("reset");
  await clip("themes", "Choose a theme.", "Preview each theme on your map before you keep it.", async () => {
    await s.driver("themeGallery", "phosphor");
    appearance.push({ kind: "theme", ...(await s.driver("state")).theme });
    await step("sleep:2200");
    await s.driver("focusTheme", "helios");
    appearance.push({ kind: "theme", ...(await s.driver("state")).theme });
    await step("sleep:2200");
    await s.driver("focusTheme", "rime");
    appearance.push({ kind: "theme", ...(await s.driver("state")).theme });
    await step("sleep:2200");
    await s.driver("theme", "rime");
    await s.driver("themeGallery", false);
  }, 2);
  await clip("environments", "Choose your surroundings separately.", "Keep your theme while you change the environment.", async () => {
    await s.driver("environmentPicker");
    await step("sleep:1800");
    await s.driver("previewEnvironment", "vault");
    await s.driver("keepEnvironment");
    await s.driver("command", "wallpaper");
    appearance.push({ kind: "environment", ...(await s.driver("state")).environment });
    await step("sleep:3200");
    await s.driver("environment", "aurora");
    appearance.push({ kind: "environment", ...(await s.driver("state")).environment });
    await step("sleep:3200");
    await s.driver("environment", "nebula");
    appearance.push({ kind: "environment", ...(await s.driver("state")).environment });
  }, 3.2);
  await s.driver("command", "wallpaper");
  await titleCard("end", "The network for SI.", "github.com/willctl/xenonet", 4);
  const list = path.join(work, "clips.txt");
  fs.writeFileSync(list, clips.map(file => `file '${file.replaceAll("\\", "/").replaceAll("'", "'\\''")}'`).join("\n"));
  const output = path.join(root, "docs", "showcase.mp4");
  // No metadata: no tags, no encoder names, no x264 SEI (it lists the encoder and its settings).
  const bare = ["-map_metadata", "-1", "-fflags", "+bitexact"];
  await encode("-f", "concat", "-safe", "0", "-i", list, "-c", "copy", "-bsf:v", "filter_units=remove_types=6", ...bare, "-map_chapters", "-1", "-metadata:s:v:0", "encoder=", "-metadata:s:a:0", "encoder=", "-movflags", "+faststart", output);
  await encode("-ss", "5", "-i", output, "-frames:v", "1", ...bare, path.join(root, "docs", "showcase-poster.png"));
  const appearanceStart = 3.5 + evidence.slice(0, 5).reduce((seconds, clip) => seconds + clip.frames / 30, 0);
  await encode("-ss", String(appearanceStart), "-t", "7", "-i", output, "-filter_complex", "fps=8,scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer:bayer_scale=5", ...bare, path.join(root, "docs", "showcase-preview.gif"));
  // Any clock or date that showed in the page's text while recording.
  const clocks = await s.eval("return window.__showcaseScrub === true ? [...window.__showcaseClocks] : null");
  const logs = await s.logs({ level: "error" });
  fs.writeFileSync(path.join(work, "capture.json"), JSON.stringify({ output, zone, clocks, clips: evidence, appearance, errors: logs.entries }, null, 2));
  if (logs.entries.length) throw new Error("The recording reported page errors; inspect .xenonet/showcase/capture.json.");
  if (!clocks) throw new Error("The page reloaded during the recording, so its clock may show; record afresh.");
  if (clocks.length) throw new Error(`The recording shows a clock or a date: ${clocks.join("; ")}`);
  console.log(`Saved ${output}`);
}
