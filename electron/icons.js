// The SVG is the source for the desktop, tray and installer art. Keep the
// mark independent of the console's themes and legible at notification size.
import fs from "node:fs";
import zlib from "node:zlib";
import { BrowserWindow, nativeImage } from "electron";

const sourceLogo = new URL("../public/brand/xenonet.svg", import.meta.url);
const logo = fs.readFileSync(fs.existsSync(sourceLogo) ? sourceLogo : new URL("../dist/brand/xenonet.svg", import.meta.url), "utf8");
const markPath = logo.match(/<path d="([^"]+)"/)[1];

const PAGE = `<!doctype html><canvas id="c"></canvas><script>
const MARK = new Path2D(${JSON.stringify(markPath)});
// The brand: the mark in xeno green on void black, text in cool off-white.
const INK = "#05070A", MARK_GREEN = "#39FF9F", PAPER = "#DCE6EE", MUTED = "#7D8C99", LINE = "#1C2632";

function mark(g, x, y, size, color) {
  g.save();
  g.translate(x, y);
  g.scale(size / 64, size / 64);
  g.fillStyle = color;
  g.fill(MARK);
  g.restore();
}

function squircle(g, x, y, size) {
  const r = size / 2;
  g.beginPath();
  for (let i = 0; i <= 360; i++) {
    const a = i * Math.PI / 180, c = Math.cos(a), s = Math.sin(a);
    const px = x + r + r * Math.sign(c) * Math.abs(c) ** 0.4;
    const py = y + r + r * Math.sign(s) * Math.abs(s) ** 0.4;
    i ? g.lineTo(px, py) : g.moveTo(px, py);
  }
  g.closePath();
}

function draw(S, working, need, mac = false) {
  const c = document.getElementById("c");
  c.width = c.height = S;
  const g = c.getContext("2d");
  const inset = mac ? S * 25 / 256 : S <= 24 ? 0 : S / 32;
  const body = S - inset * 2;
  g.save();
  if (S >= 48) {
    g.shadowColor = "#0005";
    g.shadowBlur = S * 0.04;
    g.shadowOffsetY = S * 0.018;
  }
  g.fillStyle = INK;
  if (mac) squircle(g, inset, inset, body);
  else { g.beginPath(); g.roundRect(inset, inset, body, body, body * 0.24); }
  g.fill();
  g.restore();
  // A quiet edge keeps the tile visible against a dark taskbar.
  g.strokeStyle = "#ffffff24";
  g.lineWidth = Math.max(0.5, S / 512);
  g.stroke();
  const padding = body * (S <= 24 ? 0.04 : 0.065);
  mark(g, inset + padding, inset + padding, body - 2 * padding, MARK_GREEN);
  const n = need || working;
  if (n) {
    const r = S * 0.225, x = S - r - S * 0.015, y = r + S * 0.015;
    g.beginPath(); g.arc(x, y, r + S * 0.035, 0, Math.PI * 2);
    g.fillStyle = INK; g.fill();
    g.beginPath(); g.arc(x, y, r, 0, Math.PI * 2);
    g.fillStyle = need ? "#FFB020" : MARK_GREEN; g.fill();
    g.fillStyle = INK;
    g.font = "700 " + Math.round(r * (n > 9 ? 1.1 : 1.5)) + "px Segoe UI, system-ui, sans-serif";
    g.textAlign = "center"; g.textBaseline = "middle";
    g.fillText(n > 99 ? "99" : String(n), x, y + r * 0.05);
  }
  return c.toDataURL("image/png");
}

function glyph(S, working, need) {
  const c = document.getElementById("c");
  c.width = c.height = S;
  const g = c.getContext("2d");
  mark(g, 0, 0, S, need ? "#b56d00" : "#000");
  if (working || need) {
    g.beginPath(); g.arc(S * 0.875, S * 0.125, S * 0.095, 0, Math.PI * 2);
    g.fillStyle = need ? "#b56d00" : "#000"; g.fill();
  }
  return c.toDataURL("image/png");
}

async function sheet(sizes) {
  const cell = 280, out = document.createElement("canvas");
  out.width = sizes.length * cell; out.height = 3 * cell;
  const g = out.getContext("2d");
  const images = await Promise.all(sizes.map((s) => new Promise((resolve) => {
    const im = new Image(); im.onload = () => resolve(im); im.src = draw(s, 0, 0);
  })));
  [["#0D1218", PAPER], ["#f2f4f6", INK], ["#74867b", "#fff"]].forEach(([bg, fg], row) => {
    g.fillStyle = bg; g.fillRect(0, row * cell, out.width, cell);
    images.forEach((im, i) => {
      const s = sizes[i]; g.drawImage(im, i * cell + (cell - s) / 2, row * cell + (cell - s) / 2 - 10);
      g.fillStyle = fg; g.font = "13px Segoe UI, system-ui, sans-serif"; g.textAlign = "center";
      g.fillText(s + " px", i * cell + cell / 2, (row + 1) * cell - 12);
    });
  });
  return out.toDataURL("image/png");
}

function art(kind) {
  const sidebar = kind === "sidebar", W = sidebar ? 164 : 150, H = sidebar ? 314 : 57;
  const out = document.createElement("canvas"); out.width = W; out.height = H;
  const g = out.getContext("2d");
  g.fillStyle = INK; g.fillRect(0, 0, W, H);
  if (sidebar) {
    mark(g, 26, 32, 112, MARK_GREEN);
    g.fillStyle = PAPER; g.font = "600 27px Segoe UI, system-ui, sans-serif";
    g.textAlign = "center"; g.fillText("XenoNet", W / 2, 184);
    g.fillStyle = MUTED; g.font = "12px Segoe UI, system-ui, sans-serif";
    g.fillText("SI network", W / 2, 210);
    g.fillStyle = LINE; g.fillRect(24, H - 32, W - 48, 1);
  } else {
    mark(g, 98, 6, 44, MARK_GREEN);
    g.fillStyle = PAPER; g.font = "600 21px Segoe UI, system-ui, sans-serif";
    g.fillText("XenoNet", 14, 36);
  }
  return out.toDataURL("image/png");
}

// Finder places icons at these coordinates; keep the artwork clear of them.
function dmgArt(scale) {
  const W = 660, H = 452, out = document.createElement("canvas");
  out.width = W * scale; out.height = H * scale;
  const g = out.getContext("2d"); g.scale(scale, scale);
  g.fillStyle = INK; g.fillRect(0, 0, W, H);
  mark(g, 281, 28, 48, MARK_GREEN);
  g.fillStyle = PAPER; g.font = "600 30px Segoe UI, system-ui, sans-serif";
  g.fillText("XenoNet", 337, 62);
  g.strokeStyle = MUTED; g.lineWidth = 2; g.lineJoin = "round";
  g.beginPath(); g.moveTo(298, 210); g.lineTo(360, 210);
  g.moveTo(349, 199); g.lineTo(360, 210); g.lineTo(349, 221); g.stroke();
  // These neutral plates keep Finder's black or white filenames readable.
  for (const cx of [180, 480]) {
    g.fillStyle = "#7c817d"; g.beginPath(); g.roundRect(cx - 70, 272, 140, 24, 6); g.fill();
  }
  g.textAlign = "center"; g.fillStyle = PAPER; g.font = "14px Segoe UI, system-ui, sans-serif";
  g.fillText("Drag XenoNet to Applications.", W / 2, 353);
  g.fillStyle = MUTED; g.font = "11px Segoe UI, system-ui, sans-serif";
  g.fillText("If macOS can't verify XenoNet, open System Settings > Privacy & Security > Open Anyway.", W / 2, 382);
  return out.toDataURL("image/png");
}

async function pkgArt(scale) {
  const im = new Image();
  await new Promise((resolve) => { im.onload = resolve; im.src = draw(128 * scale, 0, 0, true); });
  const out = document.createElement("canvas"); out.width = 220 * scale; out.height = 230 * scale;
  out.getContext("2d").drawImage(im, 26 * scale, 72 * scale, 128 * scale, 128 * scale);
  return out.toDataURL("image/png");
}
</script>`;

let win = null;

async function painter() {
  if (win && !win.isDestroyed()) return win;
  win = new BrowserWindow({ show: false, width: 64, height: 64, webPreferences: { sandbox: true } });
  await win.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(PAGE)}`);
  return win;
}

export async function iconDataUrl(size, working = 0, need = 0, { mac = false } = {}) {
  const w = await painter();
  return w.webContents.executeJavaScript(`draw(${size | 0}, ${working | 0}, ${need | 0}, ${!!mac})`);
}

const pngOf = (url) => Buffer.from(url.split(",")[1], "base64");

// The macOS menu bar image, 16 pt at 1x and 2x. A template (tinted by macOS)
// unless something needs you.
export async function trayGlyph(working = 0, need = 0) {
  const w = await painter();
  const image = nativeImage.createEmpty();
  for (const scaleFactor of [1, 2]) {
    const url = await w.webContents.executeJavaScript(`glyph(${16 * scaleFactor}, ${working | 0}, ${need | 0})`);
    image.addRepresentation({ scaleFactor, buffer: pngOf(url) });
  }
  image.setTemplateImage(!need);
  return image;
}

export async function dmgArtDataUrl(scale = 1) {
  const w = await painter();
  return w.webContents.executeJavaScript(`dmgArt(${scale | 0})`);
}

export async function pkgArtDataUrl(scale = 2) {
  const w = await painter();
  return w.webContents.executeJavaScript(`pkgArt(${scale | 0})`);
}

export async function sheetDataUrl(sizes) {
  const w = await painter();
  return w.webContents.executeJavaScript(`sheet(${JSON.stringify(sizes)})`);
}

export async function artDataUrl(kind) {
  const w = await painter();
  return w.webContents.executeJavaScript(`art(${JSON.stringify(kind)})`);
}

// A 24-bit bottom-up BMP, the only image format NSIS wizard pages take.
export function bmpFromImage(image) {
  const { width, height } = image.getSize();
  const bgra = image.toBitmap();
  const stride = (width * 3 + 3) & ~3;
  const out = Buffer.alloc(54 + stride * height);
  out.write("BM", 0, "ascii");
  out.writeUInt32LE(out.length, 2);
  out.writeUInt32LE(54, 10);
  out.writeUInt32LE(40, 14);
  out.writeInt32LE(width, 18);
  out.writeInt32LE(height, 22);
  out.writeUInt16LE(1, 26);
  out.writeUInt16LE(24, 28);
  out.writeUInt32LE(stride * height, 34);
  out.writeInt32LE(2835, 38);
  out.writeInt32LE(2835, 42);
  for (let y = 0; y < height; y++) {
    const row = 54 + (height - 1 - y) * stride;
    for (let x = 0; x < width; x++) {
      const s = (y * width + x) * 4, d = row + x * 3;
      out[d] = bgra[s];
      out[d + 1] = bgra[s + 1];
      out[d + 2] = bgra[s + 2];
    }
  }
  return out;
}

export async function iconImage(size, working = 0, need = 0, scaleFactor = 1) {
  const url = await iconDataUrl(size, working, need);
  const buf = Buffer.from(url.split(",")[1], "base64");
  return nativeImage.createFromBuffer(buf, { scaleFactor });
}

// A Windows .ico holding one PNG per size (supported since Vista).
export function packIco(entries) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(entries.length, 4);
  const dir = Buffer.alloc(16 * entries.length);
  let offset = 6 + dir.length;
  entries.forEach(({ size, png }, i) => {
    const o = i * 16;
    dir.writeUInt8(size >= 256 ? 0 : size, o);
    dir.writeUInt8(size >= 256 ? 0 : size, o + 1);
    dir.writeUInt8(0, o + 2);
    dir.writeUInt8(0, o + 3);
    dir.writeUInt16LE(1, o + 4);
    dir.writeUInt16LE(32, o + 6);
    dir.writeUInt32LE(png.length, o + 8);
    dir.writeUInt32LE(offset, o + 12);
    offset += png.length;
  });
  return Buffer.concat([header, dir, ...entries.map((e) => e.png)]);
}

// A macOS .icns holding one PNG per slot (supported since 10.7). The @2x
// slots reuse the PNG of twice the size.
const ICNS_SLOTS = [["icp4", 16], ["icp5", 32], ["ic11", 32], ["ic12", 64], ["ic07", 128], ["ic08", 256], ["ic13", 256], ["ic09", 512], ["ic14", 512], ["ic10", 1024]];
export function packIcns(pngBySize) {
  const chunks = ICNS_SLOTS.filter(([, size]) => pngBySize[size]).map(([type, size]) => {
    const head = Buffer.alloc(8);
    head.write(type, 0, "ascii");
    head.writeUInt32BE(pngBySize[size].length + 8, 4);
    return Buffer.concat([head, pngBySize[size]]);
  });
  const head = Buffer.alloc(8);
  head.write("icns", 0, "ascii");
  head.writeUInt32BE(8 + chunks.reduce((n, c) => n + c.length, 0), 4);
  return Buffer.concat([head, ...chunks]);
}

// Marks a PNG as `dpi` dots per inch (a pHYs chunk), so macOS shows a 2x
// image at its point size.
export function pngWithDpi(png, dpi) {
  const ppm = Math.round(dpi / 0.0254);
  const data = Buffer.alloc(9);
  data.writeUInt32BE(ppm, 0);
  data.writeUInt32BE(ppm, 4);
  data.writeUInt8(1, 8);
  const type = Buffer.from("pHYs", "ascii");
  const chunk = Buffer.alloc(21);
  chunk.writeUInt32BE(9, 0);
  type.copy(chunk, 4);
  data.copy(chunk, 8);
  chunk.writeUInt32BE(zlib.crc32(Buffer.concat([type, data])), 17);
  const ihdrEnd = 8 + 8 + 13 + 4; // signature, IHDR length+type, IHDR data, CRC
  return Buffer.concat([png.subarray(0, ihdrEnd), chunk, png.subarray(ihdrEnd)]);
}

export function closePainter() {
  if (win && !win.isDestroyed()) win.destroy();
  win = null;
}
