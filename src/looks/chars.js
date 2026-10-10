// Characters drawn once onto a texture in a grid of cells, for what draws
// text as points: the cursor form's memory cells (looks/actors.js) and the
// hex digits round a tunnel's gate (looks/gate.js). The actors layer makes
// one for each set of characters and disposes them when the theme changes.
import * as THREE from "three";

// The theme's mono font may lack a block character; these have them, on
// macOS and Windows, so one is drawn rather than an empty box.
const SYMBOLS = '"Apple Symbols", "Segoe UI Symbol", monospace';
const CELL = 64;

// { texture, count, grid }: character i sits in cell (i % grid,
// floor(i / grid)), counted from the top left, white on clear, so a
// material tints it.
export function charAtlas(text, font) {
  const chars = [...new Set([...text].filter((c) => c.trim()))].slice(0, 64);
  const grid = Math.ceil(Math.sqrt(chars.length));
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = grid * CELL;
  const g = canvas.getContext("2d");
  g.fillStyle = "#fff";
  g.textAlign = "center";
  g.textBaseline = "middle";
  // Smaller than the cell, so a mipmap never pulls in the next one.
  g.font = `500 ${Math.round(CELL * 0.62)}px ${font}, ${SYMBOLS}`;
  g.shadowColor = "rgba(255,255,255,0.6)";
  g.shadowBlur = CELL * 0.06;
  chars.forEach((ch, i) => g.fillText(ch, ((i % grid) + 0.5) * CELL, (Math.floor(i / grid) + 0.5) * CELL));
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  // Rows count down from the top, as they're drawn.
  texture.flipY = false;
  texture.anisotropy = 4;
  return { texture, count: chars.length, grid };
}

// GLSL for point sprites that draw a character from the atlas (uniforms
// atlas and grid): charAt(cell) is how much of character `cell` covers
// this fragment.
export const CHAR_SPRITE = /* glsl */ `
  uniform sampler2D atlas; uniform float grid;
  float charAt(float cell) {
    vec2 p = gl_PointCoord - 0.5;
    float k = floor(cell + 0.5);
    vec2 at = vec2(mod(k, grid), floor(k / grid));
    return texture2D(atlas, (at + p + 0.5) / grid).a;
  }
`;
