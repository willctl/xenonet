// The lines between nodes (theme.lines: style, curvature and width). The
// force graph asks for one per link and places it whenever its ends move.
// Each style draws a link as one line object, so a style costs no more
// draw calls than XenoNet's own:
//   build({ from, to, width, opacity, bow })  a link's line, from the color
//                                             at its source to the one at
//                                             its target; bow > 0 bends it
//   place(line, start, end)                   between two points
//   update(line, opacity, width, t)           each frame, as it glows with
//                                             the work flowing through it,
//                                             with the ambient clock (for
//                                             every link: allocate nothing
//                                             here)
//   dispose(line)                             frees what build made
import * as THREE from "three";
import { fatLine, writeLine, gradientColors, arcPoints, pointsArray, releaseMaterial } from "../fx.js";

const a = new THREE.Vector3(), b = new THREE.Vector3(), dir = new THREE.Vector3(), across = new THREE.Vector3(), UP = new THREE.Vector3(0, 1, 0), SIDE = new THREE.Vector3(1, 0, 0);

const set = (line, opacity, width) => {
  line.material.opacity = opacity;
  line.material.linewidth = width;
};
const drop = (line) => {
  line.geometry.dispose();
  releaseMaterial(line.material);
};

// The link's direction, and one square to it for bending it to one side.
// Returns its length.
function frame(start, end) {
  dir.set(end.x - start.x, end.y - start.y, end.z - start.z);
  const len = dir.length();
  if (len < 1e-6) return 0;
  dir.divideScalar(len);
  across.crossVectors(dir, Math.abs(dir.y) > 0.95 ? SIDE : UP).normalize();
  return len;
}

const ARC = 20, FILAMENT = 16;

export const LINES = {
  // An anti-aliased line, its color running from one end to the other,
  // straight unless it bows.
  solid: {
    build({ from, to, width, opacity, bow }) {
      const segments = bow ? 40 : 1;
      const line = fatLine(segments, { width, opacity }, gradientColors(segments, from, to));
      Object.assign(line.userData, { points: pointsArray(segments), bow });
      return line;
    },
    place(line, start, end) {
      const { points, bow } = line.userData;
      if (bow) arcPoints(a.set(start.x, start.y, start.z), b.set(end.x, end.y, end.z), points, bow);
      else {
        points[0].set(start.x, start.y, start.z);
        points[1].set(end.x, end.y, end.z);
      }
      writeLine(line, points);
    },
    update: set,
    dispose: drop,
  },

  // A curve raised over its middle, always, by the curvature's share of
  // its length (a little when that's 0): gently bowed lines, or magnetic
  // loops standing up like prominences when it's high.
  arc: {
    build({ from, to, width, opacity, bow }) {
      const line = fatLine(ARC, { width, opacity }, gradientColors(ARC, from, to));
      Object.assign(line.userData, { points: pointsArray(ARC), lift: Math.max(0.12, bow) });
      return line;
    },
    place(line, start, end) {
      const { points, lift } = line.userData;
      const len = Math.hypot(end.x - start.x, end.y - start.y, end.z - start.z);
      const cx = (start.x + end.x) / 2, cy = (start.y + end.y) / 2 + len * lift, cz = (start.z + end.z) / 2;
      for (let i = 0; i <= ARC; i++) {
        const s = i / ARC, u = 1 - s;
        points[i].set(u * u * start.x + 2 * u * s * cx + s * s * end.x, u * u * start.y + 2 * u * s * cy + s * s * end.y, u * u * start.z + 2 * u * s * cz + s * s * end.z);
      }
      writeLine(line, points);
    },
    update: set,
    dispose: drop,
  },

  // A living filament: sagging a little, beaded with brighter light along
  // its length, and pulsing slowly, each on its own beat.
  filament: {
    build({ from, to, width, opacity, bow }) {
      const colors = gradientColors(FILAMENT, from, to);
      for (let i = 0; i <= FILAMENT; i++) {
        const bead = 0.7 + 0.7 * Math.sin((Math.PI * i * 5) / FILAMENT) ** 8;
        for (let k = 0; k < 3; k++) colors[i * 3 + k] *= bead;
      }
      const line = fatLine(FILAMENT, { width, opacity }, colors);
      Object.assign(line.userData, { points: pointsArray(FILAMENT), sag: Math.max(0.04, bow), beat: Math.random() * Math.PI * 2 });
      return line;
    },
    place(line, start, end) {
      const { points, sag } = line.userData;
      const len = frame(start, end);
      for (let i = 0; i <= FILAMENT; i++) {
        const s = i / FILAMENT, down = len * sag * Math.sin(Math.PI * s), side = len * 0.02 * Math.sin(Math.PI * 4 * s);
        points[i].set(start.x + dir.x * len * s + across.x * side, start.y + dir.y * len * s - down + across.y * side, start.z + dir.z * len * s + across.z * side);
      }
      writeLine(line, points);
    },
    update(line, opacity, width, t = 0) {
      set(line, opacity * (0.7 + 0.3 * Math.sin(t * 1.6 + line.userData.beat)), width);
    },
    dispose: drop,
  },

  // A hairline, half the width, catching the light now and then like
  // frost.
  hairline: {
    build(link) {
      const line = LINES.solid.build(link);
      line.userData.glint = Math.random() * Math.PI * 2;
      return line;
    },
    place: (line, start, end) => LINES.solid.place(line, start, end),
    update(line, opacity, width, t = 0) {
      const glint = Math.max(0, Math.sin(t * 0.6 + line.userData.glint)) ** 24;
      set(line, Math.min(1, opacity * (0.85 + 1.6 * glint)), width * 0.45);
    },
    dispose: drop,
  },
};
