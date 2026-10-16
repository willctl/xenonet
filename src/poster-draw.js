// How a poster of the day is drawn: the constellation in a frame for
// sharing, the mark band, the day's archetype as its headline, the
// brightest stars by their star names, the day's numbers in one thin
// band, and the node-X with the site in a corner. No border. Redacted by
// default (core/redact.js): no project names and no clock, unless an
// export reveals more. drawPoster takes any 2D context, so the tests draw
// onto one that records; src/poster.js gives it the page's canvas.
import { tokens as compact } from "../core/units.js";
import { MARK_BAND, MARK_URL, APP_GLYPHS, drawMark, drawGlyph } from "./mark.js";

const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : "");
const rgba = (hex, a) => `rgba(${parseInt(hex.slice(1, 3), 16)},${parseInt(hex.slice(3, 5), 16)},${parseInt(hex.slice(5, 7), 16)},${a})`;

// What the poster says, from the day's constellation: the band, the
// headline (the archetype, else the date), the date, the stars, the apps
// and the stats. Nothing here is a time. `reveal.names` puts the projects'
// names in place of the star names, and `reveal.tokens` adds the tokens to
// the band; both are off unless an export turns them on.
export function posterLines(data, reveal = {}) {
  const t = data.totals ?? {};
  const names = (data.stars ?? []).slice(0, 4).map((s) => (reveal.names && s.project ? s.project : s.name)).filter(Boolean);
  const stats = [
    [t.sessions ?? 0, "sessions"],
    [t.turns ?? 0, "turns"],
    [compact(t.toolCalls ?? 0), "tool calls"],
    [t.files ?? 0, "files edited"],
    ...(reveal.tokens ? [[compact(data.tokens ?? 0), "tokens"]] : []),
  ].map(([v, k]) => [String(v), k]);
  const apps = Object.entries(data.apps ?? {})
    .filter(([app, n]) => APP_GLYPHS[app] && n > 0)
    .map(([app, n]) => ({ app, shape: APP_GLYPHS[app].shape, label: APP_GLYPHS[app].label, count: n }));
  return {
    band: MARK_BAND,
    headline: data.archetype ? cap(data.archetype) : (data.date ?? ""),
    date: data.archetype ? (data.date ?? "") : "",
    stars: { label: reveal.names ? "BUSIEST PROJECTS" : "BRIGHTEST STARS", names },
    apps,
    stats,
    url: MARK_URL,
  };
}

// Draws the poster, `width` by `height`, over `image` (the map at that
// size), in the theme's colors and fonts. Returns what it wrote.
export function drawPoster(g, { width: W, height: H, image, data, reveal = {}, theme }) {
  const { colors, fonts } = theme;
  const lines = posterLines(data, reveal);
  // Sized for a 1080 frame; a smaller frame keeps its text readable.
  const u = Math.max(0.75, Math.min(W, H) / 1080);
  const pad = 64 * u;

  if (image) g.drawImage(image, 0, 0, W, H);
  // Shades top and bottom so the text reads over the stars.
  let grd = g.createLinearGradient(0, H * 0.62, 0, H);
  grd.addColorStop(0, rgba(colors.bg, 0));
  grd.addColorStop(1, rgba(colors.bg, 0.92));
  g.fillStyle = grd;
  g.fillRect(0, H * 0.62, W, H * 0.38);
  grd = g.createLinearGradient(0, 0, 0, H * 0.22);
  grd.addColorStop(0, rgba(colors.bg, 0.75));
  grd.addColorStop(1, rgba(colors.bg, 0));
  g.fillStyle = grd;
  g.fillRect(0, 0, W, H * 0.22);

  // The band, top left.
  g.textBaseline = "alphabetic";
  g.textAlign = "left";
  g.fillStyle = colors.accent;
  g.font = `600 ${16 * u}px ${fonts.display}`;
  g.letterSpacing = `${5 * u}px`;
  g.fillText(lines.band, pad, pad + 8 * u);

  // The node-X and the site, top right, in line with the band.
  g.fillStyle = rgba(colors.label, 0.85);
  g.font = `400 ${14 * u}px ${fonts.mono}`;
  g.letterSpacing = "0px";
  const urlW = g.measureText(lines.url).width;
  const glyph = 22 * u;
  g.fillText(lines.url, W - pad - urlW, pad + 8 * u);
  drawMark(g, W - pad - urlW - glyph - 10 * u, pad + 8 * u - glyph + 3 * u, glyph, colors.accent);

  // The headline: the day's archetype, with the date under it.
  g.fillStyle = colors.bright;
  g.font = `600 ${58 * u}px ${fonts.display}`;
  g.letterSpacing = `${1 * u}px`;
  g.shadowColor = rgba(colors.accent, 0.6);
  g.shadowBlur = 24 * u;
  g.fillText(lines.headline, pad, pad + 78 * u);
  g.shadowBlur = 0;
  if (lines.date) {
    g.fillStyle = colors.muted;
    g.font = `500 ${22 * u}px ${fonts.display}`;
    g.letterSpacing = `${2 * u}px`;
    g.fillText(lines.date, pad, pad + 114 * u);
  }

  // The stars, above the band: as many as fit across.
  const bandTop = H - pad - 72 * u;
  if (lines.stars.names.length) {
    g.fillStyle = colors.label;
    g.font = `500 ${17 * u}px ${fonts.display}`;
    g.letterSpacing = `${3 * u}px`;
    const names = [...lines.stars.names];
    const text = () => `${lines.stars.label}  ·  ${names.map((n) => n.toUpperCase()).join("  ·  ")}`;
    while (names.length > 1 && g.measureText(text()).width > W - pad * 2) names.pop();
    lines.stars.shown = names;
    g.fillText(text(), pad, bandTop - 36 * u);
  }

  // Which apps the day came from, right of the stars line: a glyph and a
  // count each, from the right edge in.
  if (lines.apps.length) {
    g.font = `500 ${13 * u}px ${fonts.display}`;
    g.letterSpacing = `${3 * u}px`;
    const y = bandTop - 76 * u;
    let x = W - pad;
    for (const a of [...lines.apps].reverse()) {
      const text = `${a.label.toUpperCase()} ${a.count}`;
      x -= g.measureText(text).width;
      g.fillStyle = colors.muted;
      g.fillText(text, x, y);
      x -= 16 * u;
      drawGlyph(g, a.shape, x, y - 5 * u, 11 * u, colors.accent);
      x -= 30 * u;
    }
  }

  // The day's numbers, in one thin band along the bottom: a rule, then
  // each number with its word.
  g.strokeStyle = rgba(colors.accent, 0.35);
  g.lineWidth = 1.5 * u;
  g.beginPath();
  g.moveTo(pad, bandTop);
  g.lineTo(W - pad, bandTop);
  g.stroke();
  const colW = (W - pad * 2) / lines.stats.length;
  lines.stats.forEach(([v, k], i) => {
    const x = pad + i * colW;
    g.fillStyle = colors.bright;
    g.font = `500 ${40 * u}px ${fonts.mono}`;
    g.letterSpacing = "0px";
    g.fillText(v, x, H - pad - 22 * u);
    g.fillStyle = colors.muted;
    g.font = `500 ${13 * u}px ${fonts.display}`;
    g.letterSpacing = `${4 * u}px`;
    g.fillText(k.toUpperCase(), x, H - pad + 2 * u);
  });
  return lines;
}
