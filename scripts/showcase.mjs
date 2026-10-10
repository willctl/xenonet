// A product video made from XenoNet's synthetic workspace and real control channel.
// Run through `xenonet script run`; see docs/guide/showcase.md.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { findFfmpeg } from "../core/dirs.js";
import { prepareAudio, startAudio, stopAudio } from "./showcase-audio.mjs";

export default async function ({ xenonet: s, step }) {
  if (s.id !== "showcase") throw new Error("Use --instance showcase so recording cannot include personal sessions.");
  const ffmpeg = findFfmpeg();
  if (!ffmpeg) throw new Error("Put ffmpeg on PATH before starting the recording instance.");
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
    await encode("-loop", "1", "-i", png, "-i", audio, "-t", String(seconds), "-r", "30", "-vf", "fade=t=in:st=0:d=0.35,fade=t=out:st=" + (seconds-.35) + ":d=0.35,format=yuv420p", "-af", `apad,afade=t=in:d=0.15,afade=t=out:st=${seconds-.35}:d=0.35`, "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-c:v", "libx264", "-preset", "medium", "-crf", "19", file);
    clips.push(file);
  }
  async function clip(name, title, detail, action, seconds = 6, crop = null) {
    console.log(`Recording ${name}`);
    const raw = path.join(work, `${name}-raw.mp4`);
    const file = path.join(work, `${name}.mp4`);
    const audio = path.join(work, `${name}-audio.webm`);
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
    await encode("-i", raw, "-i", overlay, "-ss", String(audioOffset), "-i", audio, "-filter_complex", `[0:v]${framing}pad=1920:1080:160:112:color=0x05070A[frame];[frame][1:v]overlay=0:0:format=auto,format=yuv420p[out]`, "-map", "[out]", "-map", "2:a", "-af", `apad,afade=t=in:d=0.025,afade=t=out:st=${duration-.025}:d=0.025`, "-t", String(duration), "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-c:v", "libx264", "-preset", "medium", "-crf", "21", "-movflags", "+faststart", file);
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
  await titleCard("intro", "See what your\nSIs are doing.", "A local service, tools and desktop console.", 3.5);
  await clip("overview", "See each session in its project.", "Claude Code and Codex, with background SIs beside them.", async () => {}, 5);
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
  await clip("agents", "See what background SIs are doing.", "SIs use XenoNet to find peers, claim files and send messages.", () => s.driver("select", "actor:wren"), 6);
  await step("deselect");
  await clip("control", "SIs can operate the console too.", "This recording uses the same controls for finding and selecting work.", async () => {
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
  await encode("-f", "concat", "-safe", "0", "-i", list, "-c", "copy", "-movflags", "+faststart", output);
  await encode("-ss", "5", "-i", output, "-frames:v", "1", path.join(root, "docs", "showcase-poster.png"));
  const appearanceStart = 3.5 + evidence.slice(0, 5).reduce((seconds, clip) => seconds + clip.frames / 30, 0);
  await encode("-ss", String(appearanceStart), "-t", "7", "-i", output, "-filter_complex", "fps=8,scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96[p];[b][p]paletteuse=dither=bayer:bayer_scale=5", path.join(root, "docs", "showcase-preview.gif"));
  const logs = await s.logs({ level: "error" });
  fs.writeFileSync(path.join(work, "capture.json"), JSON.stringify({ output, clips: evidence, appearance, errors: logs.entries }, null, 2));
  if (logs.entries.length) throw new Error("The recording reported page errors; inspect .xenonet/showcase/capture.json.");
  console.log(`Saved ${output}`);
}
