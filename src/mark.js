// XenoNet's mark on what leaves the app: the node-X from
// public/brand/xenonet.svg, drawn onto a canvas (posters) or as inline SVG
// (films), the band that names an export, and the site. Plain data and
// drawing, no page imports, so the tests draw it too.

export const MARK_BAND = "XENONET · CONSTELLATION OF THE DAY";
export const MARK_URL = "xenonet.dev";

// The node-X as polygons in a 64 by 64 box: the centre, the four corners
// and the four bars between them. The same shapes as the SVG's path
// (test/poster.test.js checks they haven't drifted apart).
export const MARK_POLYGONS = [
  [[32, 24], [40, 32], [32, 40], [24, 32]],
  [[9, 9], [19, 9], [19, 19], [9, 19]],
  [[45, 9], [55, 9], [55, 19], [45, 19]],
  [[9, 45], [19, 45], [19, 55], [9, 55]],
  [[45, 45], [55, 45], [55, 55], [45, 55]],
  [[22.77, 19.23], [28.77, 25.23], [25.23, 28.77], [19.23, 22.77]],
  [[44.77, 22.77], [38.77, 28.77], [35.23, 25.23], [41.23, 19.23]],
  [[19.23, 41.23], [25.23, 35.23], [28.77, 38.77], [22.77, 44.77]],
  [[41.23, 44.77], [35.23, 38.77], [38.77, 35.23], [44.77, 41.23]],
];

// The mark `size` pixels wide, its top-left corner at (x, y).
export function drawMark(g, x, y, size, color) {
  const k = size / 64;
  g.fillStyle = color;
  g.beginPath();
  for (const poly of MARK_POLYGONS) {
    poly.forEach(([px, py], i) => (i ? g.lineTo(x + px * k, y + py * k) : g.moveTo(x + px * k, y + py * k)));
    g.closePath();
  }
  g.fill();
}

// The mark as inline SVG, in the current text color.
export const markSvg = (cls = "") => `<svg class="${cls}" viewBox="0 0 64 64" fill="currentColor" aria-hidden="true"><path d="${MARK_POLYGONS.map((poly) => poly.map(([x, y], i) => `${i ? "L" : "M"}${x} ${y}`).join("") + "Z").join(" ")}"/></svg>`;

// A glyph for each SI app a session or an actor runs in, so an export says
// which apps the day's constellation came from: the app's name and a shape
// drawn beside it. Drawn as a path, so it needs no font.
export const APP_GLYPHS = {
  claude: { label: "Claude Code", shape: "diamond" },
  codex: { label: "Codex", shape: "ring" },
  hermes: { label: "Hermes", shape: "triangle" },
  grok: { label: "Grok", shape: "square" },
  muse: { label: "Muse", shape: "star" },
};

// One app's glyph, `size` wide, centred at (cx, cy).
export function drawGlyph(g, shape, cx, cy, size, color) {
  const r = size / 2;
  g.fillStyle = color;
  g.strokeStyle = color;
  g.lineWidth = Math.max(1, size / 7);
  g.beginPath();
  if (shape === "ring") {
    g.arc(cx, cy, r * 0.8, 0, Math.PI * 2);
    g.stroke();
    return;
  }
  const pts =
    shape === "diamond" ? [[cx, cy - r], [cx + r, cy], [cx, cy + r], [cx - r, cy]]
    : shape === "triangle" ? [[cx, cy - r], [cx + r, cy + r * 0.8], [cx - r, cy + r * 0.8]]
    : shape === "square" ? [[cx - r * 0.8, cy - r * 0.8], [cx + r * 0.8, cy - r * 0.8], [cx + r * 0.8, cy + r * 0.8], [cx - r * 0.8, cy + r * 0.8]]
    : Array.from({ length: 10 }, (_, i) => {
        const a = -Math.PI / 2 + (i * Math.PI) / 5;
        const d = i % 2 ? r * 0.45 : r;
        return [cx + Math.cos(a) * d, cy + Math.sin(a) * d];
      });
  pts.forEach(([x, y], i) => (i ? g.lineTo(x, y) : g.moveTo(x, y)));
  g.closePath();
  g.fill();
}
