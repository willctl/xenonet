// Posters of the day: the map rendered at one of the sizes for sharing
// (core/frames.js), drawn up by poster-draw.js, as a PNG to save or copy.
// Everything happens on this computer: the constellation comes from this
// XenoNet (/api/constellation, with star names in place of project names),
// the PNG is the canvas alone, so it carries no metadata, and nothing is
// uploaded. The export panel (export-panel.js) shows a preview first.
import { THEME, themeFonts } from "./theme.js";
import { posterSize } from "../core/frames.js";
import { drawPoster } from "./poster-draw.js";

// The day's constellation, with the projects' names only when revealing
// them.
export const fetchConstellation = (reveal = {}) => fetch(`/api/constellation${reveal.names ? "?names=1" : ""}`).then((r) => r.json());

// Renders the poster at `size` (a size's id), with what `reveal` turns on
// (names, tokens), onto a canvas: { canvas, width, height, lines }.
// `data` is the constellation, fetched when not given.
export async function renderPoster({ scene, size, reveal = {}, data = null, theme = THEME }) {
  const { id, width: W, height: H } = posterSize(size);
  // Canvas text only uses font weights already loaded; load every one drawn.
  await themeFonts(theme);
  data ??= await fetchConstellation(reveal);
  const shot = scene.renderHighRes(W, H);
  const img = new Image();
  img.src = shot.url;
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const lines = drawPoster(canvas.getContext("2d"), { width: W, height: H, image: img, data, reveal, theme });
  return { canvas, width: W, height: H, size: id, lines };
}

// The PNG, as a blob: the pixels and nothing else.
export const posterBlob = ({ canvas }) => new Promise((resolve) => canvas.toBlob(resolve, "image/png"));

// Saves the poster: a download, which the desktop app puts in the
// pictures folder (electron/main.js, wireDownloads).
export async function savePoster(poster) {
  const a = document.createElement("a");
  a.href = poster.canvas.toDataURL("image/png");
  a.download = `xenonet-${new Date().toLocaleDateString("en-CA")}-${poster.width}x${poster.height}.png`;
  a.click();
  return { bytes: (await posterBlob(poster)).size };
}

// Copies the poster to the clipboard as an image.
export async function copyPoster(poster) {
  const blob = await posterBlob(poster);
  await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
  return { bytes: blob.size };
}
