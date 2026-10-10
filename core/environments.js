// The place around the map is chosen independently of its theme.
export const DEFAULT_ENVIRONMENT = "nebula";
export const ENVIRONMENTS = Object.freeze([
  { id: "nebula", name: "Nebula", about: "The original drifting clouds and star field.", shape: "clouds", index: 0 },
  { id: "aurora", name: "Aurora", about: "Slow curtains of light across a quiet atmospheric horizon.", shape: "curtains", index: 1 },
  { id: "trench", name: "Trench", about: "A deep-sea trench with rippling caustics and distant light shafts.", shape: "water", index: 2 },
  { id: "vault", name: "Vault", about: "A monumental hall of repeating arches and illuminated ribs.", shape: "arches", index: 3 },
]);
export const environmentOf = (id) => ENVIRONMENTS.find((e) => e.id === id) ?? ENVIRONMENTS[0];
