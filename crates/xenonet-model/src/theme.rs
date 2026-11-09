//! A theme (`theme/v1`): how the console looks and sounds.

use crate::num::Uint;
use crate::schema::{Document, Opt, Record};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// A color, #rrggbb.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Color(pub String);

impl JsonSchema for Color {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Color")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^#[0-9a-fA-F]{6}$", "description": "A color, #rrggbb" })
    }
}

/// A MIDI note, 0 to 127.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Midi(pub u8);

impl JsonSchema for Midi {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Midi")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "minimum": 0, "maximum": 127 })
    }
}

// An open choice: one of the named options, or any string a theme brings
// of its own.
//
// Types a field reaches through `Opt` are marked inline: `Opt<T>` shares
// `T`'s schema id, so the generator would otherwise take `T` for a
// recursion and write a `$ref`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(untagged)]
pub enum Open<T> {
    Named(T),
    Other(String),
}

// Colors by role.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "camelCase")]
pub struct Colors {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub bg: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub surface: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub surface_low: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub text: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub muted: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub dim: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub bright: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub accent: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub core: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub project: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub satellite: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub native: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub zone: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub resource: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub claude: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub codex: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub grok: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub lineage: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub collab: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub collision: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sync: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub message: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub wiki: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub attention: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub review: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stuck: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pressure: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub danger: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub gold: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub archived: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub label: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub actor: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub human: Opt<Color>,
}

// What an agent is doing, by tool family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Categories {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub shell: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub edit: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub read: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub web: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub mcp: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub agent: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub code: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub other: Opt<Color>,
}

// Context page types.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Pages {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub project: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub preference: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reference: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub howto: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub decision: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub person: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub topic: Opt<Color>,
}

// CSS font families.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Fonts {
    /// Titles, and the map's labels
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub display: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub mono: Opt<String>,
    /// Body text in the panels; the display font when left out
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub body: Opt<String>,
}

// The sky through the day; each time of day is four colors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Sky {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 4))]
    pub night: Opt<Vec<Color>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 4))]
    pub dawn: Opt<Vec<Color>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 4))]
    pub day: Opt<Vec<Color>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 4))]
    pub dusk: Opt<Vec<Color>>,
}

// How a star is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum StarShape {
    Point,
    Flake,
    Mote,
}

// The stars.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Stars {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub count: Opt<Uint>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub tints: Opt<Vec<Color>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub shape: Opt<Open<StarShape>>,
    /// Units a second; [0, -0.6, 0] is falling snow
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(extend("items" = false))]
    pub drift: Opt<(f64, f64, f64)>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub twinkle: Opt<f64>,
}

// How the grid is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum GridStyle {
    Polar,
    Hex,
    Cells,
    None,
}

// The grid on the floor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Grid {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub center: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub lines: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub opacity: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub style: Opt<Open<GridStyle>>,
}

// The lattice behind the map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Lattice {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub opacity: Opt<f64>,
}

// How the sweep is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum SweepShape {
    Beam,
    Aurora,
    Flare,
    Hand,
    Ring,
    None,
}

// The sweep across the map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Sweep {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub opacity: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub speed: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub shape: Opt<Open<SweepShape>>,
}

// The dust in the air.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Dust {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub color: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub opacity: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub size: Opt<f64>,
}

// The glow over the whole map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Glow {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub bloom: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub radius: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub threshold: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub exposure: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub fog: Opt<f64>,
}

// How a sonar ping is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum SonarStyle {
    Ring,
    Ripple,
    Ping,
    Tick,
}

// The sonar.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Sonar {
    /// Seconds between pings
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub every: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub style: Opt<Open<SonarStyle>>,
}

// The stats panel's charts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "camelCase")]
pub struct Charts {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub m1: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub m2: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub m3: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub m4: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub m5: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub other: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub cache_read: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub input: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub cache_write: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub output: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub one: Opt<Color>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub bad: Opt<Color>,
}

// An oscillator's waveform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

// The music.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Sound {
    /// A cycle of chords, as MIDI notes
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub chords: Opt<Vec<Vec<Midi>>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub scale: Opt<Vec<Midi>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub bpm: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pad: Opt<Open<Waveform>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub lead: Opt<Open<Waveform>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub drone: Opt<Midi>,
}

// How motion eases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum Ease {
    Smooth,
    Stepped,
}

// Ambient motion.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Motion {
    /// A multiplier for ambient motion; 1 is today's
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub pace: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub ease: Opt<Open<Ease>>,
    /// How much nodes swell and fade at rest
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub breathe: Opt<f64>,
}

// How a line between nodes is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum LineStyle {
    Solid,
    Dashed,
    Arc,
    Filament,
    Seam,
    Hairline,
}

// The lines between nodes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Lines {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub style: Opt<Open<LineStyle>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub curvature: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub width: Opt<f64>,
}

// An actor's form on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum ActorForm {
    Orb,
    Cursor,
    Crystal,
    Flame,
    Swarm,
    Jelly,
}

// A knob's value: a number or a switch.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Knob {
    Number(f64),
    Switch(bool),
}

impl JsonSchema for Knob {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Knob")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": ["number", "boolean"] })
    }
}

// The person at the console.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Human {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub form: Opt<Open<ActorForm>>,
}

// The glow round each actor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct ActorGlow {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub opacity: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub scale: Opt<f64>,
}

// How an actor breathes at rest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Breathe {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub period: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub depth: Opt<f64>,
}

// How a beam between actors is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum BeamStyle {
    Pulse,
    Stream,
    Bolt,
}

// The beams between actors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Beam {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub style: Opt<Open<BeamStyle>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub width: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub speed: Opt<f64>,
}

// How an actor reacts to an event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct React {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 0))]
    pub flash: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub ripple: Opt<bool>,
}

// The actors layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Actors {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub form: Opt<Open<ActorForm>>,
    /// Knobs for the form: rings, facets, tendrils
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub params: Opt<Record<Knob>>,
    /// Each agent takes one by a stable hash of its id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 1))]
    pub palette: Opt<Vec<Color>>,
    /// The person at the console
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub human: Opt<Human>,
    /// The glow round each actor
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub glow: Opt<ActorGlow>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub breathe: Opt<Breathe>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub beam: Opt<Beam>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub react: Opt<React>,
}

/// How the console looks and sounds: colors, fonts, the sky, stars, floor, sweep and sonar, the glow, the music, and the actors layer. Every field is optional: what a theme leaves out comes from the one it extends.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// Lowercase letters, digits and dashes; a file's name is its id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(regex(pattern = r"^[a-z0-9][a-z0-9-]{0,40}$"))]
    pub id: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub name: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub about: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub built_in: Opt<bool>,
    /// A theme of yours with a built-in's id: it takes that one's place
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub replaces_built_in: Opt<bool>,
    /// The theme it starts from; xenonet when left out
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub extends: Opt<String>,
    /// By role; wiki is the shared context's. The page applies only #rrggbb values. actor and human are the actors layer's defaults
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub colors: Opt<Colors>,
    /// The rings named identities' projects stand on: five colors that mean nothing else on the map. One a theme brings close to a color with a meaning is swapped for a spare
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 5, max = 5))]
    pub identities: Opt<Vec<Color>>,
    /// What an agent is doing, by tool family
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub categories: Opt<Categories>,
    /// Context page types
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pages: Opt<Pages>,
    /// CSS font families
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub fonts: Opt<Fonts>,
    /// The sky through the day: deep, primary cloud, secondary cloud, band
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sky: Opt<Sky>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stars: Opt<Stars>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub grid: Opt<Grid>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub lattice: Opt<Lattice>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sweep: Opt<Sweep>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub dust: Opt<Dust>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub glow: Opt<Glow>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sonar: Opt<Sonar>,
    /// The stats panel's charts
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub charts: Opt<Charts>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sound: Opt<Sound>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub motion: Opt<Motion>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub lines: Opt<Lines>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub actors: Opt<Actors>,
}

impl Document for Theme {
    const ID: &'static str = "theme/v1";
    const TITLE: &'static str = "Theme";
    const DESCRIPTION: &'static str = "How the console looks and sounds: colors, fonts, the sky, stars, floor, sweep and sonar, the glow, the music, and the actors layer. Every field is optional: what a theme leaves out comes from the one it extends.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Theme>];
