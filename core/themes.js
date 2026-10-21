// How the console looks and sounds: every color, the fonts, the sky through
// the day, the stars, the floor grid, the sweep and the dust, the glow, the
// sonar, the lines between nodes, the pace of things, the music, and how
// actors look. Plain data with no Node imports: the console page applies a
// theme, and the server and the xenonet command list them and keep your
// choice.
//
// `xenonet` is XenoNet's own look, xeno green on void black; every other theme names only
// what it changes. Your own themes are JSON files in XenoNet's state folder
// (themes/<id>.json) with the same fields and "extends" naming the theme
// they start from (xenonet when left out). Every field is optional, so a
// theme made before a field existed still works: it takes the value of
// the theme underneath.
export const DEFAULT_THEME = "xenonet";

const xenonet = {
  name: "XenoNet",
  about: "Xeno green on void black",
  // By role. The page's panels, the map's nodes and lines, and the alerts all
  // take their colors from here.
  colors: {
    bg: "#05070a",
    surface: "#0a1015",
    surfaceLow: "#06090d",
    text: "#dce6ee",
    muted: "#7d8c99",
    dim: "#566677",
    bright: "#e9fff4",
    accent: "#39ff9f",
    core: "#e9fff4",
    project: "#39ff9f",
    satellite: "#22c983",
    native: "#b18cff",
    zone: "#7d8aa3",
    resource: "#ff7ad9",
    claude: "#ff9d5c",
    codex: "#5fe1ff",
    grok: "#c6ff3a",
    lineage: "#ffd76a",
    collab: "#ff5fd2",
    collision: "#ff3b6b",
    sync: "#7fd8ff",
    message: "#c8ffe4",
    wiki: "#c9b8ff",
    attention: "#ffb547",
    review: "#7fe7ff",
    stuck: "#ff7a3d",
    pressure: "#ffc857",
    danger: "#ff5d73",
    gold: "#ffd76a",
    archived: "#4a5868",
    label: "#a8d8c0",
    // Actors (see actors below): an agent in general, and you at the console.
    actor: "#c9a0ff",
    human: "#e9fff4",
  },
  // The rings named identities' projects stand on (see ringColors), five
  // colors that mean nothing else on the map.
  identities: ["#8fa8ff", "#ff8fb3", "#f2ead3", "#7ef0dd", "#b69bff"],
  // What an agent is doing, by tool family.
  categories: { shell: "#7cf7ff", edit: "#ffd76a", read: "#8ab4ff", web: "#c792ff", mcp: "#ff7ad9", agent: "#ffffff", code: "#5dffc1", other: "#9fb3c8" },
  // Wiki page types.
  pages: { project: "#7fe7ff", preference: "#ffd76a", reference: "#c9b8ff", howto: "#7dffb2", decision: "#ffa36b", person: "#ff9be4", topic: "#e8f4ff" },
  // Titles and the map's labels (display), code and numbers (mono), and the
  // panels' body text (body), which is the display font unless a theme
  // names one, so a theme that only changes its display font changes both.
  fonts: { display: '"Chakra Petch", "Segoe UI", sans-serif', mono: '"JetBrains Mono", ui-monospace, "Cascadia Mono", monospace' },
  // The sky through the day: [deep, primary cloud, secondary cloud, band].
  sky: {
    night: ["#05080c", "#0b2a20", "#101c2c", "#0c2622"],
    dawn: ["#0a0b0c", "#2a2a14", "#14262a", "#30301a"],
    day: ["#06100e", "#06372c", "#102a36", "#16362e"],
    dusk: ["#080a10", "#1e1a3a", "#0c2a2a", "#2a1a2a"],
  },
  // The stars: how many, their tints, what they're drawn as (points,
  // falling flakes or drifting motes), how flakes and motes drift (units a
  // second; [0, -4, 0] is falling snow) and how much they twinkle (0 to 1).
  stars: { count: 3200, tints: ["#b8ffe0", "#ffffff", "#9fe6ff", "#d9e6ee"], shape: "point", drift: [0, 0, 0], twinkle: 0.5 },
  // The floor under the map (polar, hex, cells or none), the icosahedral
  // lattice around it, the sweep that circles the floor (beam, aurora,
  // flare or none; hand and ring are drawn as the beam for now), how
  // strongly it shows (XenoNet's faint beam is 0.03) and how fast it turns,
  // and the dust in the air.
  grid: { style: "polar", center: "#1d7a55", lines: "#0c3326", opacity: 0.35 },
  lattice: { opacity: 0.035 },
  sweep: { shape: "beam", opacity: 0.03, speed: 0.35 },
  dust: { color: "#39ff9f", opacity: 0.3, size: 1.6 },
  glow: { bloom: 0.95, radius: 0.55, threshold: 0.55, exposure: 1.05, fog: 0.0007 },
  // The wave the core sends out (ring, ripple or ping; tick is drawn as the
  // ring for now), every so many seconds; 0 or less sends none (see
  // sonarGap).
  sonar: { style: "ring", every: 16 },
  // The lines between nodes (solid, arc, filament or hairline; dashed and
  // seam are drawn solid for now), how far they bow (0 is straight; an arc
  // always bows a little) and their width (1 is XenoNet's).
  lines: { style: "solid", curvature: 0, width: 1 },
  // Ambient motion: its pace (1 is XenoNet's), smooth or stepped (in ticks),
  // and how much nodes breathe at rest (0 to 1; at 0.1 sessions bob as
  // they always have).
  motion: { pace: 1, ease: "smooth", breathe: 0.1 },
  // Chart colors for the stats panel, checked on the panel's surface with
  // the dataviz rules (see src/stats.js).
  charts: { m1: "#0da1bd", m2: "#d37431", m3: "#9c77e8", m4: "#b18b0a", m5: "#d957b6", other: "#4d6272", cacheRead: "#066f83", input: "#0b96af", cacheWrite: "#31bddb", output: "#74e3fe", one: "#7b95a8", bad: "#ec835a" },
  // The music: a chord cycle (MIDI notes), the scale melodies come from,
  // the tempo, and the voices.
  sound: {
    chords: [
      [50, 57, 61, 64, 66],
      [47, 54, 57, 62, 64],
      [43, 50, 54, 59, 61],
      [45, 52, 59, 61, 64],
    ],
    scale: [62, 64, 66, 68, 69, 71, 73],
    bpm: 84,
    pad: "sawtooth",
    lead: "triangle",
    drone: 38,
  },
  // Actors: the always-on agents beside the map and you at the console (see
  // docs/design/contracts.md). The form agents take (orb, cursor, crystal,
  // flame, swarm or jelly) and its knobs, their colors (each agent takes
  // one by a hash of its id; you take colors.human), your form, and how
  // they glow, breathe, beam into what they act on (pulse, stream or bolt)
  // and react.
  actors: {
    form: "orb",
    params: {},
    palette: ["#39ff9f", "#5fe1ff", "#ffd76a", "#ff7ad9", "#b18cff", "#ff9d5c"],
    human: { form: "orb" },
    glow: { opacity: 0.5, scale: 2.4 },
    breathe: { period: 4.5, depth: 0.12 },
    beam: { style: "pulse", width: 1.4, speed: 1 },
    react: { flash: 1.6, ripple: true },
  },
};

const THEMES = {
  xenonet,
  phosphor: {
    name: "Phosphor",
    about: "Amber phosphor on black glass, like an old monochrome terminal",
    colors: {
      bg: "#020200",
      surface: "#0f0b02",
      surfaceLow: "#060400",
      text: "#ffcf70",
      muted: "#a8822e",
      dim: "#765923",
      bright: "#fff3cc",
      accent: "#ffb000",
      core: "#fff3cc",
      project: "#ffb000",
      satellite: "#d08a00",
      native: "#ff8a3d",
      zone: "#7a6a4a",
      resource: "#ffd27a",
      claude: "#ffd27a",
      codex: "#6dff8a",
      grok: "#e45cff",
      lineage: "#ffe08a",
      collab: "#ff9a5a",
      collision: "#ff4a3a",
      sync: "#ffd88a",
      message: "#fff1c2",
      wiki: "#e8c88a",
      attention: "#fff1c2",
      review: "#ffe08a",
      stuck: "#ff8a3d",
      pressure: "#ffe08a",
      danger: "#ff4a3a",
      gold: "#ffe08a",
      archived: "#4a3a1a",
      label: "#ffc966",
      actor: "#ff9a5a",
      human: "#fff3cc",
    },
    // Its questions are cream, so that ring is lime.
    identities: ["#8fa8ff", "#ff8fb3", "#c3f584", "#7ef0dd", "#b69bff"],
    categories: { shell: "#ffd27a", edit: "#ffe08a", read: "#ffb000", web: "#ff9a5a", mcp: "#ffcf70", agent: "#fff3cc", code: "#6dff8a", other: "#a8822e" },
    pages: { project: "#ffd27a", preference: "#ffe08a", reference: "#e8c88a", howto: "#6dff8a", decision: "#ff8a3d", person: "#ffb07a", topic: "#fff3cc" },
    fonts: { display: '"JetBrains Mono", ui-monospace, monospace', mono: '"JetBrains Mono", ui-monospace, monospace' },
    sky: {
      night: ["#020200", "#1a1202", "#120a02", "#2a1a04"],
      dawn: ["#030200", "#241802", "#160c02", "#342406"],
      day: ["#030200", "#20160a", "#140c04", "#302008"],
      dusk: ["#030200", "#221400", "#140a02", "#321e04"],
    },
    stars: { count: 1600, tints: ["#ffd27a", "#ffffff", "#ffb000", "#fff1c2"] },
    grid: { center: "#6a4400", lines: "#2a1a00", opacity: 0.55 },
    lattice: { opacity: 0.045 },
    sweep: { opacity: 0.05, speed: 0.5 },
    dust: { color: "#ffb000", opacity: 0.22, size: 1.4 },
    glow: { bloom: 1.25, radius: 0.4, threshold: 0.45, exposure: 1.0, fog: 0.0008 },
    sound: {
      // A minor pentatonic, square-wave blips over a quiet pad.
      chords: [
        [45, 52, 57, 60, 64],
        [41, 48, 53, 57, 60],
        [48, 55, 60, 64, 67],
        [43, 50, 55, 59, 62],
      ],
      scale: [57, 60, 62, 64, 67, 69, 72],
      bpm: 96,
      pad: "triangle",
      lead: "square",
      drone: 33,
    },
    // A blinking block cursor in a ring of memory cells.
    actors: { form: "cursor", palette: ["#ffb000", "#6dff8a", "#ffd27a", "#ff9a5a"] },
  },
  ochre: {
    name: "Ochre",
    about: "Dunes under a bruised sky: ochre, rust and the blue of a desert night",
    colors: {
      bg: "#070402",
      surface: "#170d06",
      surfaceLow: "#0b0603",
      text: "#f3dcc0",
      muted: "#a08466",
      dim: "#725b45",
      bright: "#fff4e0",
      accent: "#e8a85a",
      core: "#fff4e0",
      project: "#e8a85a",
      satellite: "#c8883e",
      native: "#c87a50",
      zone: "#8a7a6a",
      resource: "#ff8ac0",
      claude: "#ff7a3a",
      codex: "#5aa8ff",
      grok: "#c25cff",
      lineage: "#ffd76a",
      collab: "#ff7aa0",
      collision: "#ff3a3a",
      sync: "#8ac8ff",
      message: "#ffe6c8",
      wiki: "#d8b8ff",
      attention: "#ffd24a",
      review: "#9ad0ff",
      stuck: "#ff6a3a",
      pressure: "#ffb84a",
      danger: "#ff5a4a",
      gold: "#f2c46a",
      archived: "#4a3a2a",
      label: "#e8c8a0",
      actor: "#ff8ac0",
      human: "#fff4e0",
    },
    // Its Codex is blue, so that ring is lime.
    identities: ["#c3f584", "#ff8fb3", "#f2ead3", "#7ef0dd", "#b69bff"],
    categories: { shell: "#f2c46a", edit: "#ffd76a", read: "#9ad0ff", web: "#d8b8ff", mcp: "#ff8ac0", agent: "#ffffff", code: "#5aa8ff", other: "#a08466" },
    pages: { project: "#9ad0ff", preference: "#ffd76a", reference: "#d8b8ff", howto: "#7dd8a0", decision: "#ff8a5a", person: "#ff9be4", topic: "#fff4e0" },
    fonts: { display: '"Avenir Next", "Futura", "Segoe UI", "Chakra Petch", sans-serif' },
    sky: {
      night: ["#06040a", "#1a1030", "#2a1220", "#24162a"],
      dawn: ["#140804", "#7a3a10", "#5a1a18", "#8a4a1a"],
      day: ["#120a04", "#5a3410", "#3a1a10", "#6a4418"],
      dusk: ["#100604", "#6a2a10", "#4a1424", "#7a3a14"],
    },
    stars: { count: 2600, tints: ["#ffe6c8", "#ffffff", "#ffb07a", "#9ac8ff"] },
    grid: { center: "#6a4a20", lines: "#2a1a0a", opacity: 0.35 },
    sweep: { opacity: 0.04, speed: 0.28 },
    dust: { color: "#e8a85a", opacity: 0.45, size: 1.8 },
    glow: { bloom: 0.85, radius: 0.6, threshold: 0.6, exposure: 1.1, fog: 0.0008 },
    sound: {
      // E phrygian dominant, slow, over a low drone.
      chords: [
        [40, 47, 52, 56, 59],
        [41, 48, 53, 57, 60],
        [38, 45, 50, 53, 57],
        [40, 47, 52, 56, 62],
      ],
      scale: [64, 65, 68, 69, 71, 72, 74],
      bpm: 72,
      drone: 28,
    },
    // A dust devil.
    actors: { form: "swarm", palette: ["#e8a85a", "#5aa8ff", "#ff8ac0", "#ffd76a"] },
  },
  gridline: {
    name: "Gridline",
    about: "Light trails on an endless grid, in electric cyan and orange on black",
    colors: {
      bg: "#000004",
      surface: "#00080e",
      surfaceLow: "#000308",
      text: "#e6fbff",
      muted: "#6aa0b4",
      dim: "#366276",
      bright: "#ffffff",
      accent: "#00e5ff",
      core: "#ffffff",
      project: "#00e5ff",
      satellite: "#00b8d0",
      native: "#d07aff",
      zone: "#5a7a8a",
      resource: "#ff6ad0",
      claude: "#ff9a2a",
      codex: "#9cff4a",
      grok: "#3dffb0",
      lineage: "#ffd23a",
      collab: "#ff4fd8",
      collision: "#ff1a4a",
      sync: "#6ad8ff",
      message: "#d8fbff",
      wiki: "#c8b8ff",
      attention: "#ffb000",
      review: "#8af6ff",
      stuck: "#ff7a1a",
      pressure: "#ffd23a",
      danger: "#ff2a5a",
      gold: "#ffd23a",
      archived: "#2a3a44",
      label: "#8ad8f0",
      actor: "#d07aff",
      human: "#ffffff",
    },
    identities: ["#8fa8ff", "#ff8fb3", "#f2ead3", "#7ef0dd", "#b69bff"],
    categories: { shell: "#00e5ff", edit: "#ffd23a", read: "#6ad8ff", web: "#d07aff", mcp: "#ff6ad0", agent: "#ffffff", code: "#9cff4a", other: "#6aa0b4" },
    pages: { project: "#8af6ff", preference: "#ffd23a", reference: "#c8b8ff", howto: "#9cff4a", decision: "#ff9a2a", person: "#ff6ad0", topic: "#e6fbff" },
    sky: {
      night: ["#000206", "#001018", "#06001a", "#001a22"],
      dawn: ["#020206", "#0a1018", "#14041a", "#101a22"],
      day: ["#000408", "#00141c", "#08021c", "#002026"],
      dusk: ["#020208", "#0a0a1c", "#16021c", "#0a1424"],
    },
    stars: { count: 900, tints: ["#8af6ff", "#ffffff", "#ffb06a", "#c8f8ff"] },
    grid: { center: "#00b8d0", lines: "#004a5a", opacity: 0.7 },
    lattice: { opacity: 0.06 },
    sweep: { opacity: 0.06, speed: 0.6 },
    dust: { color: "#00e5ff", opacity: 0.2, size: 1.4 },
    // The bloom used to be reset every frame, so this one never showed;
    // drawn as written, 1.3 washed the grid out.
    glow: { bloom: 1.1, radius: 0.45, threshold: 0.4, exposure: 1.1, fog: 0.0006 },
    sound: {
      // A minor, four on the floor's worth of arpeggio.
      chords: [
        [45, 52, 57, 60, 64],
        [41, 48, 53, 57, 60],
        [48, 55, 60, 64, 67],
        [43, 50, 55, 59, 62],
      ],
      scale: [57, 59, 60, 62, 64, 65, 67],
      bpm: 104,
      lead: "square",
      drone: 33,
    },
    // A rezzed polyhedron.
    actors: { form: "crystal", palette: ["#00e5ff", "#ff9a2a", "#9cff4a", "#d07aff"] },
  },
  helios: {
    name: "Helios",
    about: "The sun's surface up close: white-gold light, plasma orange and magnetic magenta loops",
    colors: {
      bg: "#0a0300",
      surface: "#1c0b02",
      surfaceLow: "#120601",
      text: "#fff1db",
      muted: "#b1855f",
      dim: "#8a5b39",
      bright: "#fffbe8",
      accent: "#ffb02e",
      core: "#fffbe8",
      project: "#ffb02e",
      satellite: "#e08f1a",
      native: "#ff8fc8",
      zone: "#a88a74",
      resource: "#ff6aa8",
      claude: "#ff6a2a",
      codex: "#7fd4ff",
      grok: "#5cff5c",
      lineage: "#ffd166",
      collab: "#ff5fc0",
      collision: "#ff2aa0",
      sync: "#9fdcff",
      message: "#fff3c8",
      wiki: "#ffc8a0",
      attention: "#fff36a",
      review: "#a8e8ff",
      stuck: "#ff4a1a",
      pressure: "#ffc83a",
      danger: "#ff2d6a",
      gold: "#ffd166",
      archived: "#4a2a18",
      label: "#ffd9ad",
      actor: "#ff6aa8",
      human: "#fffbe8",
    },
    // Coronal light: magenta loops, ultraviolet, a green flash, white gold
    // and sky.
    identities: ["#ff9ee6", "#c4a8ff", "#9ef0e8", "#f4f8c8", "#a8b8ff"],
    categories: { shell: "#ffd166", edit: "#fff36a", read: "#7fd4ff", web: "#ff8fc8", mcp: "#ff6aa8", agent: "#ffffff", code: "#ffb02e", other: "#c49a80" },
    pages: { project: "#ffb02e", preference: "#fff36a", reference: "#ffc8a0", howto: "#7fd4ff", decision: "#ff6a2a", person: "#ff8fc8", topic: "#fff1db" },
    fonts: { display: '"Futura", "Avenir Next Condensed", "Segoe UI", "Chakra Petch", sans-serif' },
    sky: {
      night: ["#0a0200", "#3d0e02", "#4a0a1e", "#5a1a04"],
      dawn: ["#120400", "#6a2004", "#5a0c24", "#8a3a06"],
      day: ["#100400", "#5a1a02", "#4a0c1a", "#7a3004"],
      dusk: ["#0e0300", "#5a1004", "#500a2a", "#7a2404"],
    },
    stars: { count: 500, tints: ["#fff3d6", "#ffd166", "#ff9a4a", "#ffffff"], twinkle: 0.85 },
    grid: { style: "cells", center: "#8a3a08", lines: "#4a1a06", opacity: 0.4 },
    lattice: { opacity: 0.03 },
    sweep: { shape: "flare", opacity: 0.035, speed: 0.22 },
    dust: { color: "#ffb02e", opacity: 0.55, size: 1.8 },
    glow: { bloom: 1.1, radius: 0.62, threshold: 0.48, exposure: 1.05, fog: 0.0007 },
    sonar: { style: "ripple", every: 9 },
    lines: { style: "arc", curvature: 0.42, width: 1.1 },
    // Quick and flickering.
    motion: { pace: 1.5, breathe: 0.15 },
    charts: { m1: "#b17000", m2: "#bc5694", m3: "#068bc0", m4: "#ca5747", m5: "#07957c", other: "#6e594b", cacheRead: "#8e5300", input: "#bb731b", cacheWrite: "#e39849", output: "#ffc48a", one: "#a48d7e", bad: "#f07f77" },
    sound: {
      // C lydian, quick, with a bright saw lead.
      chords: [
        [48, 55, 59, 64, 67],
        [50, 57, 62, 66, 69],
        [43, 50, 54, 59, 62],
        [45, 52, 55, 59, 60],
      ],
      scale: [72, 74, 76, 78, 79, 81, 83],
      bpm: 108,
      pad: "sawtooth",
      lead: "sawtooth",
      drone: 36,
    },
    // A small star whose corona flares when it acts.
    actors: {
      form: "flame",
      params: { corona: 1.4, flares: true },
      palette: ["#ffb02e", "#7fd4ff", "#ff6aa8", "#fff36a", "#ff6a2a", "#ffc8a0"],
      glow: { opacity: 0.6, scale: 2.8 },
      breathe: { period: 2.6, depth: 0.16 },
      beam: { style: "stream", width: 1.5, speed: 1.4 },
      react: { flash: 2.1, ripple: true },
    },
  },
  rime: {
    name: "Rime",
    about: "Frost on black glass: glacier blue and white under a faint aurora",
    colors: {
      bg: "#01050a",
      surface: "#08151f",
      surfaceLow: "#040b12",
      text: "#e8f7ff",
      muted: "#7994a7",
      dim: "#4b6c80",
      bright: "#ffffff",
      accent: "#a6ecff",
      core: "#ffffff",
      project: "#a6ecff",
      satellite: "#7cc4e0",
      native: "#c8b8ff",
      zone: "#8a9cad",
      resource: "#e8a8ff",
      claude: "#ffb59a",
      codex: "#8affd6",
      grok: "#9dff7a",
      lineage: "#efe6bf",
      collab: "#ff9ad8",
      collision: "#ff4f6f",
      sync: "#b8e0ff",
      message: "#d8f4ff",
      wiki: "#c8d8ff",
      attention: "#fff2a6",
      review: "#c8ffe8",
      stuck: "#ff8a5a",
      pressure: "#ffe08a",
      danger: "#ff5c7d",
      gold: "#efe6bf",
      archived: "#2e3e4c",
      label: "#c8ecfa",
      actor: "#c8b8ff",
      human: "#ffffff",
    },
    // The aurora's blue, pink, green and violet, and rose frost.
    identities: ["#9fa8ff", "#ff9fc8", "#c8f59f", "#ecb8ff", "#f1daee"],
    categories: { shell: "#a6ecff", edit: "#efe6bf", read: "#b8c8ff", web: "#d8b8ff", mcp: "#f0b0e8", agent: "#ffffff", code: "#8affd6", other: "#8aa0b2" },
    pages: { project: "#a6ecff", preference: "#efe6bf", reference: "#c8b8ff", howto: "#8affd6", decision: "#ffb59a", person: "#f0b0e8", topic: "#e8f7ff" },
    fonts: { display: '"Avenir Next", "Helvetica Neue", "Segoe UI", sans-serif' },
    sky: {
      night: ["#01040a", "#06203a", "#0a3a3a", "#102a4a"],
      dawn: ["#04060c", "#1a2a44", "#2a2440", "#3a3a4a"],
      day: ["#020810", "#0a2a40", "#0c3038", "#1a3a50"],
      dusk: ["#03050c", "#14204a", "#1a1a40", "#2a2a50"],
    },
    stars: { count: 1800, tints: ["#ffffff", "#d8f4ff", "#a6ecff", "#c8d8ff"], shape: "flake", drift: [0.6, -4, 0.3], twinkle: 0.4 },
    grid: { style: "hex", center: "#3a7a9a", lines: "#123246", opacity: 0.45 },
    lattice: { opacity: 0.05 },
    sweep: { shape: "aurora", opacity: 0.035, speed: 0.12 },
    dust: { color: "#d8f4ff", opacity: 0.35, size: 1.3 },
    glow: { bloom: 0.85, radius: 0.5, threshold: 0.6, exposure: 1.05, fog: 0.0006 },
    sonar: { style: "ring", every: 24 },
    lines: { style: "hairline", curvature: 0, width: 1 },
    // Very slow, and quiet.
    motion: { pace: 0.5, breathe: 0.1 },
    charts: { m1: "#078ead", m2: "#c85b32", m3: "#049675", m4: "#886acf", m5: "#977e05", other: "#4d616b", cacheRead: "#046f87", input: "#2c94b0", cacheWrite: "#58bad6", output: "#80e1fe", one: "#8096a1", bad: "#ee8266" },
    sound: {
      // E minor 9, glassy sines with bell notes.
      chords: [
        [40, 47, 50, 54, 55],
        [36, 43, 47, 52, 55],
        [45, 52, 55, 59, 60],
        [43, 50, 54, 59, 62],
      ],
      scale: [76, 78, 79, 81, 83, 84, 86],
      bpm: 60,
      pad: "sine",
      lead: "sine",
      drone: 28,
    },
    // An ice crystal that grows facets as it works.
    actors: {
      form: "crystal",
      params: { facets: 6, grow: true },
      palette: ["#a6ecff", "#8affd6", "#c8b8ff", "#ffb59a", "#efe6bf", "#e8a8ff"],
      breathe: { period: 7, depth: 0.06 },
      beam: { style: "pulse", width: 1.0, speed: 0.6 },
      react: { flash: 1.3, ripple: true },
    },
  },
  trench: {
    name: "Trench",
    about: "The deep sea at night: black water, and the cold light of things that glow on their own",
    colors: {
      bg: "#000507",
      surface: "#021118",
      surfaceLow: "#010a0f",
      text: "#d8fbff",
      muted: "#6c949d",
      dim: "#426c76",
      bright: "#f0ffff",
      accent: "#2ef2e0",
      core: "#e8fffd",
      project: "#2ef2e0",
      satellite: "#1fb8ad",
      native: "#b49cff",
      zone: "#6a8a94",
      resource: "#ff9a6a",
      claude: "#ff7ab6",
      codex: "#9cff6a",
      grok: "#c733ff",
      lineage: "#f2d06a",
      collab: "#e07aff",
      collision: "#ff3a6a",
      sync: "#6ad8ff",
      message: "#c8fff6",
      wiki: "#a8c8ff",
      attention: "#ffd84a",
      review: "#8fe8ff",
      stuck: "#ff7a4a",
      pressure: "#ffc34a",
      danger: "#ff4a5e",
      gold: "#f2d06a",
      archived: "#1f3a42",
      label: "#a8f0f0",
      actor: "#c8a0ff",
      human: "#e8fffd",
    },
    // More things that glow in the dark.
    identities: ["#7f9cff", "#e0a8ff", "#f0f0a8", "#ffa8e0", "#c8ffe0"],
    categories: { shell: "#2ef2e0", edit: "#f2d06a", read: "#6ad8ff", web: "#b49cff", mcp: "#ff9a6a", agent: "#ffffff", code: "#9cff6a", other: "#7f9fa8" },
    pages: { project: "#2ef2e0", preference: "#f2d06a", reference: "#b49cff", howto: "#9cff6a", decision: "#ff9a6a", person: "#ff7ab6", topic: "#d8fbff" },
    fonts: { display: '"Avenir Next", "Seravek", "Segoe UI", sans-serif' },
    sky: {
      night: ["#000306", "#00202a", "#001018", "#003040"],
      dawn: ["#000508", "#002a34", "#00141e", "#003a48"],
      day: ["#00070a", "#00303a", "#001820", "#004050"],
      dusk: ["#000408", "#00222e", "#08101e", "#003040"],
    },
    stars: { count: 2400, tints: ["#c8fff6", "#ffffff", "#2ef2e0", "#ff7ab6"], shape: "mote", drift: [0.3, -1.6, 0.1], twinkle: 0.6 },
    grid: { style: "polar", center: "#0a5a66", lines: "#06323a", opacity: 0.4 },
    lattice: { opacity: 0.02 },
    sweep: { shape: "beam", opacity: 0.025, speed: 0.1 },
    dust: { color: "#2ef2e0", opacity: 0.3, size: 1.5 },
    glow: { bloom: 1.1, radius: 0.75, threshold: 0.45, exposure: 0.95, fog: 0.0011 },
    sonar: { style: "ping", every: 14 },
    lines: { style: "filament", curvature: 0.06, width: 1 },
    // Buoyant, drifting.
    motion: { pace: 0.7, breathe: 0.45 },
    charts: { m1: "#0a938e", m2: "#8f68cb", m3: "#56932b", m4: "#c0558d", m5: "#a77601", other: "#4a6367", cacheRead: "#02736e", input: "#079a94", cacheWrite: "#41c1ba", output: "#6ee8e1", one: "#7e989c", bad: "#ef806f" },
    sound: {
      // C minor, slow and low, like whale song.
      chords: [
        [36, 43, 48, 51, 55],
        [32, 39, 44, 48, 51],
        [41, 48, 53, 56, 60],
        [43, 50, 55, 58, 62],
      ],
      scale: [55, 56, 58, 60, 62, 63, 65],
      bpm: 56,
      pad: "triangle",
      lead: "sine",
      drone: 24,
    },
    // A jellyfish whose bell pulses as it breathes, trailing tendrils to
    // what it touches.
    actors: {
      form: "jelly",
      params: { tendrils: 5, pulse: true },
      palette: ["#2ef2e0", "#ff7ab6", "#9cff6a", "#c8a0ff", "#f2d06a", "#6ad8ff"],
      glow: { opacity: 0.55, scale: 2.6 },
      breathe: { period: 4, depth: 0.2 },
      beam: { style: "stream", width: 1.2, speed: 0.7 },
    },
  },
  eclipse: {
    name: "Eclipse",
    about: "Totality: black and white with one ring of light, and color only where something needs you, in hues color-blind eyes tell apart",
    colors: {
      bg: "#000000",
      surface: "#0e0e0e",
      surfaceLow: "#060606",
      text: "#ffffff",
      muted: "#b8b8b8",
      dim: "#8c8c8c",
      bright: "#ffffff",
      accent: "#ffffff",
      core: "#ffffff",
      project: "#e6e6e6",
      satellite: "#b3b3b3",
      native: "#cfcfcf",
      zone: "#8f8f8f",
      resource: "#d9d9d9",
      // Okabe and Ito's colors, which stay apart for color-blind eyes.
      claude: "#e69f00",
      codex: "#56b4e9",
      grok: "#c84dff",
      lineage: "#dddddd",
      collab: "#0072b2",
      collision: "#cc79a7",
      sync: "#a6a6a6",
      message: "#ffffff",
      wiki: "#c4c4c4",
      attention: "#f0e442",
      review: "#009e73",
      stuck: "#d55e00",
      pressure: "#f0e442",
      danger: "#d55e00",
      gold: "#f0e442",
      archived: "#3d3d3d",
      label: "#e6e6e6",
      actor: "#d6d6d6",
      human: "#ffffff",
    },
    // In color, since who owns what is worth knowing; no gray is far enough
    // from its grays.
    identities: ["#8fa8ff", "#c3f584", "#f5a3ff", "#7ef0dd", "#b69bff"],
    categories: { shell: "#ffffff", edit: "#e6e6e6", read: "#bdbdbd", web: "#a6a6a6", mcp: "#d0d0d0", agent: "#ffffff", code: "#f2f2f2", other: "#8f8f8f" },
    pages: { project: "#e6e6e6", preference: "#cfcfcf", reference: "#bdbdbd", howto: "#d9d9d9", decision: "#f0f0f0", person: "#c4c4c4", topic: "#ffffff" },
    fonts: { display: '-apple-system, "SF Pro Display", "Segoe UI", sans-serif' },
    sky: {
      night: ["#000000", "#050505", "#0a0a0a", "#161616"],
      dawn: ["#000000", "#080808", "#0c0c0c", "#1a1a1a"],
      day: ["#000000", "#0a0a0a", "#0e0e0e", "#1c1c1c"],
      dusk: ["#000000", "#070707", "#0b0b0b", "#181818"],
    },
    stars: { count: 300, tints: ["#ffffff", "#ffffff", "#cccccc", "#ffffff"], twinkle: 0 },
    grid: { style: "polar", center: "#4a4a4a", lines: "#1e1e1e", opacity: 0.6 },
    lattice: { opacity: 0.025 },
    sweep: { shape: "none" },
    dust: { color: "#ffffff", opacity: 0.1, size: 1.2 },
    glow: { bloom: 0.4, radius: 0.35, threshold: 0.75, exposure: 1.0, fog: 0.0004 },
    sonar: { style: "ring", every: 30 },
    lines: { style: "solid", curvature: 0, width: 0.9 },
    // Reduced motion, for focus and for sharing a screen.
    motion: { pace: 0.35, breathe: 0 },
    charts: { m1: "#b17000", m2: "#01976d", m3: "#c85b32", m4: "#0689c8", m5: "#c0558d", other: "#5d5d5d", cacheRead: "#0e6a9b", input: "#3e8ec1", cacheWrite: "#65b4e9", output: "#a4d8fe", one: "#929292", bad: "#eb8656" },
    sound: {
      // Plain sines, C major pentatonic.
      chords: [
        [48, 55, 60, 62, 64],
        [45, 52, 55, 60, 64],
        [43, 50, 55, 57, 62],
        [38, 45, 50, 55, 60],
      ],
      scale: [60, 62, 64, 67, 69, 72, 74],
      bpm: 70,
      pad: "sine",
      lead: "sine",
      drone: 36,
    },
    // An eclipse: a black disc with a white corona.
    actors: {
      form: "orb",
      params: { eclipse: true, rings: 0 },
      palette: ["#e69f00", "#56b4e9", "#009e73", "#cc79a7", "#f0e442", "#d55e00"],
      glow: { opacity: 0.6, scale: 1.8 },
      breathe: { period: 6, depth: 0 },
      beam: { style: "pulse", width: 1.2, speed: 0.6 },
      react: { flash: 1.0, ripple: false },
    },
  },
};

const isObject = (v) => v && typeof v === "object" && !Array.isArray(v);

// `over` on top of `base`, field by field; arrays and values replace.
export function merge(base, over) {
  const out = { ...base };
  for (const [k, v] of Object.entries(over ?? {})) out[k] = isObject(v) && isObject(base?.[k]) ? merge(base[k], v) : v;
  return out;
}

// The values XenoNet knows for the fields that name a style. Others may come
// later without a new version of the format, so a value XenoNet doesn't know
// isn't an error: it's drawn as the theme underneath draws it.
const FORMS = ["orb", "cursor", "crystal", "flame", "swarm", "jelly"];
const VOICES = ["sine", "square", "sawtooth", "triangle"];
export const STYLES = {
  "motion.ease": ["smooth", "stepped"],
  "stars.shape": ["point", "flake", "mote"],
  "grid.style": ["polar", "hex", "cells", "none"],
  "sweep.shape": ["beam", "aurora", "flare", "hand", "ring", "none"],
  "sonar.style": ["ring", "ripple", "ping", "tick"],
  "lines.style": ["solid", "dashed", "arc", "filament", "seam", "hairline"],
  "actors.form": FORMS,
  "actors.human.form": FORMS,
  "actors.beam.style": ["pulse", "stream", "bolt"],
  "sound.pad": VOICES,
  "sound.lead": VOICES,
};
// Lists that must keep their length: the sky's four colors, a drift's three
// axes and the five identity rings.
const TUPLES = new Set(["sky.night", "sky.dawn", "sky.day", "sky.dusk", "stars.drift", "identities"]);
const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i;
const isHex = (v) => typeof v === "string" && HEX.test(v);
const sixDigits = (v) => (v.length === 4 ? `#${[...v.slice(1)].map((c) => c + c).join("")}` : v).toLowerCase();
const isNumber = (v) => typeof v === "number" && Number.isFinite(v);

// Whether every item of a list fits the kind of item base's list holds.
function listFits(base, list, path) {
  if (!Array.isArray(list) || !list.length || (TUPLES.has(path) && list.length !== base.length)) return false;
  const like = base[0];
  if (Array.isArray(like)) return list.every((x) => Array.isArray(x) && x.length && x.every(isNumber));
  if (isHex(like)) return list.every(isHex);
  if (typeof like === "number") return list.every(isNumber);
  return list.every((x) => typeof x === typeof like);
}

// `value` if it can stand in for `base` at `path`, else undefined.
function fitting(base, value, path) {
  if (isObject(base)) return isObject(value) ? conform(base, value, path) : undefined;
  if (Array.isArray(base)) return listFits(base, value, path) ? value.map((x) => (isHex(x) ? sixDigits(x) : x)) : undefined;
  if (typeof base === "number") return isNumber(value) ? value : undefined;
  if (typeof base === "boolean") return typeof value === "boolean" ? value : undefined;
  if (typeof value !== "string") return undefined;
  if (isHex(base)) return isHex(value) ? sixDigits(value) : undefined;
  if (STYLES[path]) return STYLES[path].includes(value) ? value : undefined;
  return value;
}

// A theme's fonts: a list of font names, quoted or not, which the page
// puts in style attributes as well as its stylesheet. Nothing that could
// end the declaration it's in, or start another (; : { } ( ) < > \ /).
const FONTS = new Set(["fonts.display", "fonts.mono", "fonts.body"]);
const isFontList = (v) => typeof v === "string" && v.trim() !== "" && /^[\p{L}\p{M}\p{N} ,'"._+&-]+$/u.test(v);

// `over` on top of `base`, as merge does, but only where a value fits: a
// color (#rrggbb, or #rgb) for a color, a number for a number, a list of
// the same kind for a list, a style XenoNet knows for a style. Anything else
// keeps base's, so a slip in a theme file costs that one value rather than
// the page. Fields XenoNet doesn't use pass through as they are, except that
// an actor form's knobs (actors.params) are numbers and true or false, and
// the fonts (fonts.body too, which XenoNet's own leaves to the display font)
// are lists of font names.
export function conform(base, over, at = "") {
  if (!isObject(over)) return base;
  const out = { ...base };
  for (const [k, v] of Object.entries(over)) {
    // A name every object has (__proto__, constructor, toString) is never a
    // theme's field, and setting __proto__ would change what it inherits.
    if (k in Object.prototype) continue;
    const path = at ? `${at}.${k}` : k;
    if (path === "actors.params") out[k] = isObject(v) ? Object.fromEntries(Object.entries(v).filter(([, x]) => isNumber(x) || typeof x === "boolean")) : base[k];
    else if (FONTS.has(path)) {
      if (isFontList(v)) out[k] = v;
    } else if (base?.[k] === undefined) out[k] = v;
    else out[k] = fitting(base[k], v, path) ?? base[k];
  }
  return out;
}

// A theme with every field XenoNet draws: what it lacks, or gets wrong,
// taken from XenoNet's own. For themes kept from before a field existed.
export const complete = (theme) => conform(xenonet, theme);

// How many seconds apart the core's sonar waves go, for a theme's
// sonar.every, or null for none: 0 or less is how a theme turns the sonar
// off (conform keeps it, as it keeps any number). Never under a second
// apart, since each wave is new geometry that lives about six seconds
// (src/scene.js).
export const sonarGap = (every) => (isNumber(every) && every > 0 ? Math.max(1, every) : null);

// The built-in themes, and any of your own ({ id: theme }), as complete
// themes: each on top of what it extends, down to xenonet.
//
// A theme of yours with a built-in's id takes its place, and starts from
// that built-in unless it extends another; it's marked as yours
// (builtIn: false) and as replacing one. Built-ins only ever start from
// built-ins, so a file of yours never changes another built-in. A theme of
// yours that extends an id you've replaced starts from your version.
//
// Only a theme is a theme: an id is looked up among the themes' own, so
// "extends": "toString" (a name every object has) starts from XenoNet's like
// any id that isn't there, rather than from something no theme is, which
// would let every field through unchecked. The themes come back in an
// object with nothing inherited, so looking one up by id anywhere else is
// as safe.
export function themes(custom = {}) {
  const builtIn = new Map(Object.entries(THEMES).map(([id, t]) => [id, id === DEFAULT_THEME ? xenonet : conform(xenonet, t)]));
  const isBuiltIn = (id) => builtIn.has(id);
  const isYours = (id) => Object.hasOwn(custom, id);
  const yours = new Map();
  const resolve = (id, seen) => {
    if (!isYours(id)) return builtIn.get(id) ?? null;
    if (yours.has(id)) return yours.get(id);
    // A loop of themes extending each other ends at XenoNet's own.
    if (seen.has(id)) return null;
    seen.add(id);
    const t = custom[id];
    const from = typeof t.extends === "string" ? t.extends : isBuiltIn(id) ? id : DEFAULT_THEME;
    const base = (from === id ? builtIn.get(id) : resolve(from, seen)) ?? xenonet;
    // Named for its file, unless it says or takes the name of the one it replaces.
    const theme = conform(base, isBuiltIn(id) || t.name ? t : { name: id, ...t });
    yours.set(id, theme);
    return theme;
  };
  const ids = [...new Set([...builtIn.keys(), ...Object.keys(custom)])];
  const out = Object.create(null);
  for (const id of ids) {
    const mine = isYours(id);
    out[id] = { ...resolve(id, new Set()), id, builtIn: !mine, ...(mine && isBuiltIn(id) ? { replacesBuiltIn: true } : {}) };
  }
  return out;
}

export const BUILT_IN = Object.keys(THEMES);

// A color (#rrggbb) as linear red, green and blue, 0 to 1.
const linear = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));

// How far apart two colors (#rrggbb) are in brightness, as WCAG 2 measures
// contrast: from 1 (the same) to 21 (black on white). Text wants 4.5 or
// more; large text, marks and alerts 3.
export function contrast(a, b) {
  const luminance = (hex) => {
    const [r, g, bl] = linear(hex);
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

// How different two colors (#rrggbb) look: the distance between them in
// CIELAB (ΔE 1976). About 2 is just noticeable; past 20 they're plainly
// different colors.
export function colorDistance(a, b) {
  const lab = (hex) => {
    const [r, g, bl] = linear(hex);
    const f = (t) => (t > 216 / 24389 ? Math.cbrt(t) : (841 / 108) * t + 4 / 29);
    const x = f((0.4124 * r + 0.3576 * g + 0.1805 * bl) / 0.95047), y = f(0.2126 * r + 0.7152 * g + 0.0722 * bl), z = f((0.0193 * r + 0.1192 * g + 0.9505 * bl) / 1.08883);
    return [116 * y - 16, 500 * (x - y), 200 * (y - z)];
  };
  const p = lab(a), q = lab(b);
  return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

// The rings named identities' projects stand on (src/identities.js), in
// colors that mean nothing else on the map: never close to a project, an
// agent, uncommitted work, a question waiting or a collision in the theme
// in use. The theme's five (identities), which a theme that leaves them out
// takes from the one it extends, down to XenoNet's. Each is kept unless the
// theme has one of those colors near it, as a theme of yours that changes
// its colors might; then the first spare that's clear of them, and of the
// other rings, takes its place, so a name keeps its place from theme to
// theme. Built-in themes pick five that need no swapping.
const RINGS = xenonet.identities;
const RING_SPARES = ["#c3f584", "#f5a3ff", "#9fd0ff", "#ffc29e", "#d6c2ff", "#a8ffd1", "#ffe0f0", "#e2f5a8", "#ffffff"];
const MEANINGFUL = ["project", "satellite", "claude", "codex", "grok", "gold", "pressure", "attention", "danger", "collision", "stuck", "lineage"];
const CLEAR_OF_MEANING = 20, CLEAR_OF_RINGS = 15;
export function ringColors(theme) {
  const meaning = MEANINGFUL.map((k) => theme.colors?.[k]).filter(isHex).map(sixDigits);
  const own = Array.isArray(theme.identities) && theme.identities.length === RINGS.length && theme.identities.every(isHex) ? theme.identities.map(sixDigits) : RINGS;
  const out = [];
  const clear = (c) => meaning.every((m) => colorDistance(c, m) >= CLEAR_OF_MEANING) && out.every((r) => colorDistance(c, r) >= CLEAR_OF_RINGS);
  for (const ring of own) out.push(clear(ring) ? ring : (RING_SPARES.find((c) => !out.includes(c) && clear(c)) ?? ring));
  return out;
}
