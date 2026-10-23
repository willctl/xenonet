// theme/v1: a theme (core/themes.js), as a file of your own in the state
// folder's themes/ has it, and as GET /api/theme sends one resolved.
//
// Every field is optional: a theme names only what it changes, and the rest
// comes from what it extends, down to xenonet. So a resolved theme, with
// every field, fits this too, and every theme file people already have
// stays valid as tokens are added (contracts, section 4). The tokens after
// `sound` are the new ones; their values are open sets, since a value the
// console doesn't know falls back to XenoNet's.
import { define } from "./registry.js";
import { z, obj, hex, openEnum } from "./common.js";

const colors = (names) => obj(Object.fromEntries(names.map((n) => [n, hex.optional()])));
const unit = z.number().min(0).max(1);

export const Theme = define(
  "theme",
  1,
  obj({
    id: z.string().regex(/^[a-z0-9][a-z0-9-]{0,40}$/).optional().describe("Lowercase letters, digits and dashes; a file's name is its id"),
    name: z.string().optional(),
    about: z.string().optional(),
    builtIn: z.boolean().optional(),
    replacesBuiltIn: z.boolean().optional().describe("A theme of yours with a built-in's id: it takes that one's place"),
    extends: z.string().optional().describe("The theme it starts from; xenonet when left out"),

    colors: colors([
      "bg", "surface", "surfaceLow", "text", "muted", "dim", "bright", "accent",
      "core", "project", "satellite", "native", "zone", "resource", "claude", "codex", "grok",
      "lineage", "collab", "collision", "sync", "message", "wiki", "attention", "review",
      "stuck", "pressure", "danger", "gold", "archived", "label",
      "actor", "human",
    ])
      .optional()
      .describe("By role; wiki is the shared context's. The page applies only #rrggbb values. actor and human are the actors layer's defaults"),
    identities: z
      .array(hex)
      .length(5)
      .optional()
      .describe("The rings named identities' projects stand on: five colors that mean nothing else on the map. One a theme brings close to a color with a meaning is swapped for a spare"),
    categories: colors(["shell", "edit", "read", "web", "mcp", "agent", "code", "other"]).optional().describe("What an agent is doing, by tool family"),
    pages: colors(["project", "preference", "reference", "howto", "decision", "person", "topic"]).optional().describe("Context page types"),
    fonts: obj({
      display: z.string().optional().describe("Titles, and the map's labels"),
      mono: z.string().optional(),
      body: z.string().optional().describe("Body text in the panels; the display font when left out"),
    })
      .optional()
      .describe("CSS font families"),
    sky: obj(Object.fromEntries(["night", "dawn", "day", "dusk"].map((k) => [k, z.array(hex).min(4).optional()])))
      .optional()
      .describe("The sky through the day: deep, primary cloud, secondary cloud, band"),
    stars: obj({
      count: z.int().min(0).optional(),
      tints: z.array(hex).optional(),
      shape: openEnum(["point", "flake", "mote"]).optional(),
      drift: z.tuple([z.number(), z.number(), z.number()]).optional().describe("Units a second; [0, -0.6, 0] is falling snow"),
      twinkle: z.number().min(0).optional(),
    }).optional(),
    grid: obj({ center: hex.optional(), lines: hex.optional(), opacity: unit.optional(), style: openEnum(["polar", "hex", "cells", "none"]).optional() }).optional(),
    lattice: obj({ opacity: unit.optional() }).optional(),
    sweep: obj({
      opacity: unit.optional(),
      speed: z.number().optional(),
      shape: openEnum(["beam", "aurora", "flare", "hand", "ring", "none"]).optional(),
    }).optional(),
    dust: obj({ color: hex.optional(), opacity: unit.optional(), size: z.number().min(0).optional() }).optional(),
    glow: obj({
      bloom: z.number().min(0).optional(),
      radius: z.number().min(0).optional(),
      threshold: z.number().min(0).optional(),
      exposure: z.number().min(0).optional(),
      fog: z.number().min(0).optional(),
    }).optional(),
    sonar: obj({ every: z.number().min(0).optional().describe("Seconds between pings"), style: openEnum(["ring", "ripple", "ping", "tick"]).optional() }).optional(),
    charts: colors(["m1", "m2", "m3", "m4", "m5", "other", "cacheRead", "input", "cacheWrite", "output", "one", "bad"]).optional().describe("The stats panel's charts"),
    sound: obj({
      chords: z.array(z.array(z.int().min(0).max(127))).optional().describe("A cycle of chords, as MIDI notes"),
      scale: z.array(z.int().min(0).max(127)).optional(),
      bpm: z.number().positive().optional(),
      pad: openEnum(["sine", "square", "sawtooth", "triangle"]).optional(),
      lead: openEnum(["sine", "square", "sawtooth", "triangle"]).optional(),
      drone: z.int().min(0).max(127).optional(),
    }).optional(),

    motion: obj({
      pace: z.number().min(0).optional().describe("A multiplier for ambient motion; 1 is today's"),
      ease: openEnum(["smooth", "stepped"]).optional(),
      breathe: unit.optional().describe("How much nodes swell and fade at rest"),
    }).optional(),
    lines: obj({
      style: openEnum(["solid", "dashed", "arc", "filament", "seam", "hairline"]).optional(),
      curvature: z.number().optional(),
      width: z.number().min(0).optional(),
    }).optional(),
    actors: obj({
      form: openEnum(["orb", "cursor", "crystal", "flame", "swarm", "jelly"]).optional(),
      params: z.record(z.string(), z.union([z.number(), z.boolean()])).optional().describe("Knobs for the form: rings, facets, tendrils"),
      palette: z.array(hex).min(1).optional().describe("Each agent takes one by a stable hash of its id"),
      human: obj({ form: openEnum(["orb", "cursor", "crystal", "flame", "swarm", "jelly"]).optional() }).optional().describe("The person at the console"),
      glow: obj({ opacity: unit.optional(), scale: z.number().min(0).optional() }).optional().describe("The glow round each actor"),
      breathe: obj({ period: z.number().positive().optional(), depth: unit.optional() }).optional(),
      beam: obj({ style: openEnum(["pulse", "stream", "bolt"]).optional(), width: z.number().min(0).optional(), speed: z.number().min(0).optional() }).optional(),
      react: obj({ flash: z.number().min(0).optional(), ripple: z.boolean().optional() }).optional(),
    }).optional(),
  }),
  {
    title: "Theme",
    description: "How the console looks and sounds: colors, fonts, the sky, stars, floor, sweep and sonar, the glow, the music, and the actors layer. Every field is optional: what a theme leaves out comes from the one it extends.",
  },
);
