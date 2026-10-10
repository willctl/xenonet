// How the map's surroundings are drawn, by style: the stars, the floor
// grid, the sweep that circles the floor, the sonar the core sends out, and
// the lines between nodes. A theme names a style for each (core/themes.js:
// stars.shape, grid.style, sweep.shape, sonar.style, lines.style), and each
// registry maps those names to what draws them. The first in each is the
// look XenoNet has always had, and draws any style this XenoNet doesn't have
// yet. scene.js builds them, moves them each frame and disposes them when
// the theme changes, so a new style is one more entry in its registry.
export { STARS } from "./stars.js";
export { GRID } from "./grid.js";
export { SWEEP } from "./sweep.js";
export { SONAR } from "./sonar.js";
export { LINES } from "./lines.js";

// The style named, or the registry's first.
export const pick = (registry, name) => registry[name] ?? Object.values(registry)[0];
