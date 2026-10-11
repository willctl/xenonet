// The repository's social card (1280 by 640, a JPEG), the image X and other sites
// show for a link to XenoNet: the demo map on the right, the mark and the
// tagline on the left. Run through `xenonet script run` in the showcase
// instance, whose data is made up:
//   node cli/xenonet.js script run scripts/social-card.mjs --instance showcase --hidden
import fs from "node:fs";
import path from "node:path";

export default async function ({ xenonet: s, step }) {
  if (s.id !== "showcase") throw new Error("Use --instance showcase so the card can't include personal sessions.");
  const root = path.resolve(import.meta.dirname, "..");
  const mark = fs.readFileSync(path.join(root, "public", "brand", "xenonet.svg"), "utf8").match(/<path d="([^"]+)"/)[1];
  await s.window({ size: null });
  await s.driver("themeGallery", false);
  await step("theme:xenonet");
  await step("environment:nebula");
  await step("mask:on");
  await step("deselect");
  await step("reset");
  // The map alone (wallpaper mode hides the panels), from a real capture:
  // a WebGL canvas reads back blank. It reaches the page in pieces, since a
  // control request carries at most 64 KiB.
  await s.driver("command", "wallpaper");
  const shot = await s.screenshot({ name: "social-map", inline: true, maxWidth: 1600, format: "jpeg", quality: 88 });
  await s.driver("command", "wallpaper");
  fs.rmSync(path.join(root, shot.file), { force: true });
  const map = `data:${shot.image.mimeType};base64,${shot.image.data}`;
  await s.eval(`window.__cardMap = ""; return true`);
  for (let i = 0; i < map.length; i += 60000) await s.eval(`window.__cardMap += ${JSON.stringify(map.slice(i, i + 60000))}; return true`);
  const card = await s.eval(`return (async () => {
    await document.fonts.load('600 64px "Chakra Petch"');
    const c = document.createElement("canvas"); c.width = 1280; c.height = 640;
    const g = c.getContext("2d");
    g.fillStyle = "#05070A"; g.fillRect(0, 0, 1280, 640);
    const src = window.__cardMap;
    if (src) {
      const im = new Image(); im.src = src; await im.decode();
      // The map below the alerts strip, filling the right two thirds.
      const sy = im.height * 0.12, sh = im.height - sy;
      const h = 640, w = h * im.width / sh;
      g.drawImage(im, 0, sy, im.width, sh, 1280 - w * 0.86, 0, w, h);
      const fade = g.createLinearGradient(330, 0, 760, 0);
      fade.addColorStop(0, "#05070A"); fade.addColorStop(1, "rgba(5,7,10,0)");
      g.fillStyle = fade; g.fillRect(0, 0, 760, 640);
      g.fillStyle = "#05070A"; g.fillRect(0, 0, 330, 640);
    }
    g.save(); g.translate(72, 150); g.scale(1.25, 1.25); g.fillStyle = "#39FF9F"; g.fill(new Path2D(${JSON.stringify(mark)})); g.restore();
    g.fillStyle = "#DCE6EE"; g.font = '600 64px "Chakra Petch", sans-serif';
    g.fillText("XenoNet", 72, 330);
    g.fillStyle = "#39FF9F"; g.font = '500 34px "Chakra Petch", sans-serif';
    g.fillText("The network for SI.", 72, 386);
    g.fillStyle = "#9FB0BD"; g.font = '400 22px "Chakra Petch", sans-serif';
    g.fillText("See every Claude Code and Codex session on one live map.", 72, 440);
    g.fillStyle = "#5F6F7C"; g.font = '400 20px "JetBrains Mono", monospace';
    g.fillText("xenonet.dev  ·  open source", 72, 560);
    delete window.__cardMap;
    return c.toDataURL("image/jpeg", 0.92).split(",")[1];
  })()`);
  const out = path.join(root, "docs", "social-card.jpg");
  fs.writeFileSync(out, Buffer.from(card, "base64"));
  console.log(`Saved ${path.relative(root, out)}`);
}
