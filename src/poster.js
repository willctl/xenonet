// Poster mode: the constellation at 4K with the day's numbers, as a PNG, in
// the theme's colors and fonts.
import { compact } from "./telemetry.js";
import { THEME, themeFonts, withAlpha } from "./theme.js";

export async function makePoster({ scene, width = 3840 }) {
  const { colors, fonts } = THEME;
  // Canvas text only uses font weights already loaded; load every one we draw.
  await themeFonts({ fonts });
  const day = await fetch("/api/daylog").then((r) => r.json());
  const live = await fetch("/api/live").then((r) => r.json());
  const shot = scene.renderHighRes(width);
  const img = new Image();
  img.src = shot.url;
  await img.decode();

  const W = shot.width, H = shot.height, u = W / 1920;
  const c = document.createElement("canvas");
  c.width = W;
  c.height = H;
  const g = c.getContext("2d");
  g.drawImage(img, 0, 0);

  // Legibility gradients top and bottom.
  let grd = g.createLinearGradient(0, H * 0.62, 0, H);
  grd.addColorStop(0, withAlpha(colors.bg, 0));
  grd.addColorStop(1, withAlpha(colors.bg, 0.92));
  g.fillStyle = grd;
  g.fillRect(0, H * 0.62, W, H * 0.38);
  grd = g.createLinearGradient(0, 0, 0, H * 0.22);
  grd.addColorStop(0, withAlpha(colors.bg, 0.75));
  grd.addColorStop(1, withAlpha(colors.bg, 0));
  g.fillStyle = grd;
  g.fillRect(0, 0, W, H * 0.22);

  const pad = 72 * u;
  g.textBaseline = "alphabetic";
  g.fillStyle = colors.accent;
  g.font = `600 ${18 * u}px ${fonts.display}`;
  g.letterSpacing = `${6 * u}px`;
  g.fillText("XENONET · CONSTELLATION OF THE DAY", pad, pad + 10 * u);
  g.fillStyle = colors.bright;
  g.font = `600 ${58 * u}px ${fonts.display}`;
  g.letterSpacing = `${1 * u}px`;
  g.shadowColor = withAlpha(colors.accent, 0.6);
  g.shadowBlur = 24 * u;
  g.fillText(day.date, pad, pad + 78 * u);
  g.shadowBlur = 0;

  const stats = [
    [day.totals.sessions, "sessions"],
    [day.totals.turns, "turns"],
    [compact(day.totals.toolCalls), "tool calls"],
    [day.totals.files, "files edited"],
    [compact(day.tokens), "tokens processed"],
    [day.busiestHour != null ? `${day.busiestHour}:00` : "—", "busiest hour"],
  ];
  const colW = (W - pad * 2) / stats.length;
  stats.forEach(([v, k], i) => {
    const x = pad + i * colW;
    g.strokeStyle = withAlpha(colors.accent, 0.5);
    g.lineWidth = 2 * u;
    g.beginPath();
    g.moveTo(x, H - pad - 70 * u);
    g.lineTo(x, H - pad);
    g.stroke();
    g.fillStyle = colors.bright;
    g.font = `500 ${46 * u}px ${fonts.mono}`;
    g.letterSpacing = "0px";
    g.fillText(String(v), x + 18 * u, H - pad - 22 * u);
    g.fillStyle = colors.muted;
    g.font = `500 ${15 * u}px ${fonts.display}`;
    g.letterSpacing = `${4 * u}px`;
    g.fillText(k.toUpperCase(), x + 18 * u, H - pad + 2 * u);
  });

  if (day.topProjects?.length) {
    g.fillStyle = colors.label;
    g.font = `500 ${18 * u}px ${fonts.display}`;
    g.letterSpacing = `${3 * u}px`;
    g.fillText(`BUSIEST PROJECTS  ·  ${day.topProjects.slice(0, 4).map((p) => p.name.toUpperCase()).join("  ·  ")}`, pad, H - pad - 120 * u);
  }
  g.fillStyle = withAlpha(colors.label, 0.6);
  g.font = `400 ${14 * u}px ${fonts.mono}`;
  g.letterSpacing = "0px";
  const right = `${live.sessions.length} sessions · ${new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
  g.fillText(right, W - pad - g.measureText(right).width, pad + 10 * u);

  const url = c.toDataURL("image/png");
  const a = document.createElement("a");
  a.href = url;
  a.download = `xenonet-${new Date().toLocaleDateString("en-CA")}.png`;
  a.click();
  return { url, width: W, height: H };
}
