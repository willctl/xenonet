// Records a window to an MP4: every frame Chromium paints, at a steady frame
// rate (the last frame repeats when nothing new was painted, so the film
// keeps time), JPEG-encoded and piped to ffmpeg. Films of the day use it
// (src/film.js): from the palette in the desktop app, and from xenonet film
// through XenoNet Dev's control channel. It needs ffmpeg on the PATH.
import { execFileSync, spawn } from "node:child_process";
import { screen } from "electron";
import fs from "node:fs";
import path from "node:path";
import { findFfmpeg } from "../core/dirs.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));


// An H.264 encoder that keeps up with the film as it plays: the Mac's own
// (a turning map full of twinkling stars is costly to compress, and x264
// at its usual settings runs at half speed), else x264 at its fast end.
function encoder(ffmpeg) {
  let list = "";
  try {
    list = execFileSync(ffmpeg, ["-hide_banner", "-encoders"], { encoding: "utf8", timeout: 5000, windowsHide: true });
  } catch {}
  if (/\bh264_videotoolbox\b/.test(list)) return ["-c:v", "h264_videotoolbox", "-b:v", "14M", "-maxrate", "20M"];
  if (/\blibx264\b/.test(list)) return ["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"];
  return ["-c:v", "mpeg4", "-q:v", "3"];
}

export function createRecorder(getWindow) {
  let rec = null;

  // `size`: the film's frame in pixels ([1920, 1080]); the window takes the
  // size in points that paints exactly that many pixels on its display (half
  // as many on a Retina screen) while it records, and gets its own size back
  // when the recording stops, unless `keep` says to stay at the film's (for
  // a screenshot of the same frame after it). `width`: the film's width
  // (height follows).
  async function start({ file, fps = 30, width = 1920, size = null, keep = false } = {}) {
    if (rec) throw new Error("a recording is already running");
    const win = getWindow();
    if (!win) throw new Error("there's no window to record");
    const ffmpeg = findFfmpeg();
    if (!ffmpeg) throw new Error("films need ffmpeg. Install it (brew install ffmpeg, or from ffmpeg.org) and try again");
    let before = null;
    if (size) {
      before = { size: win.getContentSize(), min: win.getMinimumSize() };
      const scale = screen.getDisplayMatching(win.getBounds()).scaleFactor || 1;
      // Below the window's usual minimum, if that's what it takes.
      win.setMinimumSize(1, 1);
      win.setContentSize(Math.round(size[0] / scale), Math.round(size[1] / scale));
      await sleep(400);
    }
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const proc = spawn(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", "-f", "image2pipe", "-framerate", String(fps), "-c:v", "mjpeg", "-i", "-", "-vf", `scale=${width}:-2:flags=lanczos,format=yuv420p`, ...encoder(ffmpeg), "-movflags", "+faststart", file], { stdio: ["pipe", "ignore", "pipe"], windowsHide: true });
    let err = "";
    proc.stderr.on("data", (d) => (err = (err + d).slice(-4000)));
    proc.stdin.on("error", () => {});
    let latest = null, jpeg = null, frames = 0, size0 = null;
    win.webContents.beginFrameSubscription(false, (image) => {
      latest = image;
      jpeg = null;
    });
    win.webContents.invalidate();
    const startedAt = Date.now();
    // Each tick writes as many frames as the clock says are due.
    const timer = setInterval(() => {
      if (!latest) return;
      jpeg ??= latest.toJPEG(92);
      size0 ??= latest.getSize();
      const due = Math.floor(((Date.now() - startedAt) * fps) / 1000);
      for (let k = 0; frames < due && k < 4 && proc.stdin.writable; k++, frames++) proc.stdin.write(jpeg);
    }, 1000 / fps / 2);
    rec = { proc, timer, file, startedAt, win, frames: () => frames, err: () => err, size: () => size0, before: keep ? null : before };
    return { file, fps, width, startedAt };
  }

  async function stop() {
    if (!rec) throw new Error("nothing is being recorded");
    const r = rec;
    rec = null;
    clearInterval(r.timer);
    if (!r.win.isDestroyed()) {
      r.win.webContents.endFrameSubscription();
      if (r.before) {
        r.win.setContentSize(...r.before.size);
        r.win.setMinimumSize(...r.before.min);
      }
    }
    r.proc.stdin.end();
    const code = await new Promise((resolve) => r.proc.on("close", resolve));
    if (code !== 0) throw new Error(`ffmpeg stopped (${code}): ${r.err().trim().split("\n").pop()}`);
    return { file: r.file, frames: r.frames(), seconds: Math.round((Date.now() - r.startedAt) / 100) / 10, bytes: fs.statSync(r.file).size, captured: r.size() };
  }

  return {
    start,
    stop,
    get recording() {
      return !!rec;
    },
  };
}
