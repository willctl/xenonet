//! A computer's load (`system/v1`): CPU, memory and GPUs now and lately,
//! and what the parts are.

use crate::num::{Ms, Uint};
use crate::schema::{Document, Nullable, Opt};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

macro_rules! number {
    ($(#[$m:meta])* $name:ident => $schema:tt) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub f64);
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!($schema)
            }
        }
        impl From<f64> for $name {
            fn from(n: f64) -> Self {
                $name(n)
            }
        }
    };
}

number! {
    /// A share, 0 to 1.
    Share => { "type": "number", "minimum": 0, "maximum": 1, "description": "A share, 0 to 1" }
}
number! {
    /// A size in bytes.
    Bytes => { "type": "number", "minimum": 0, "description": "Bytes" }
}

// The processor, now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Cpu {
    #[schemars(with = "Opt<Option<String>>", length(max = 200))]
    pub model: Option<String>,
    pub threads: Uint,
    #[schemars(with = "Opt<Option<f64>>", range(min = 0))]
    pub ghz: Option<f64>,
    /// All of its threads, since the last reading
    #[schemars(range(min = 0, max = 1))]
    pub usage: f64,
    /// Each thread's use; left out of a reading from another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 1024))]
    pub cores: Opt<Vec<Share>>,
}

// Memory, now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Memory {
    pub total: Bytes,
    pub used: Bytes,
}

// One GPU, now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub index: Uint,
    #[schemars(length(max = 200))]
    pub name: String,
    pub usage: Nullable<Share>,
    /// Its own memory in use; for a GPU that shares the computer's memory (Apple's), the memory it uses of that
    pub mem_used: Nullable<Bytes>,
    /// Its own memory; for a shared one, the computer's
    pub mem_total: Nullable<Bytes>,
    /// °C, where the reader has it (NVIDIA's)
    #[schemars(with = "Opt<Option<f64>>")]
    pub temp: Option<f64>,
    /// It shares the computer's memory, as Apple's GPUs do
    pub shared: bool,
}

// One moment of the last few minutes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Moment {
    pub at: Ms,
    pub cpu: Nullable<Share>,
    pub gpu: Nullable<Share>,
    pub mem: Nullable<Share>,
}

// The processor, as a part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HardwareCpu {
    #[schemars(with = "Opt<Option<String>>", length(max = 200))]
    pub model: Option<String>,
    /// Logical processors
    pub threads: Uint,
    #[schemars(with = "Opt<Option<f64>>", range(min = 0))]
    pub ghz: Option<f64>,
}

// Memory, as a part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HardwareMemory {
    pub total: Bytes,
}

// One GPU, as a part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HardwareGpu {
    #[schemars(length(max = 200))]
    pub name: String,
    pub mem_total: Nullable<Bytes>,
    pub shared: bool,
}

/// What the parts are, from what the readers read: no names, serials or OS version
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hardware {
    /// process.platform: darwin, linux, win32
    #[schemars(length(max = 32))]
    pub platform: String,
    #[schemars(with = "Opt<Option<String>>", length(max = 32))]
    pub arch: Option<String>,
    pub cpu: HardwareCpu,
    pub memory: HardwareMemory,
    #[schemars(length(max = 16))]
    pub gpus: Vec<HardwareGpu>,
}

// One computer's reading, without the other machines' readings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reading {
    pub at: Ms,
    #[schemars(length(max = 32))]
    pub platform: String,
    #[schemars(with = "Opt<Option<String>>", length(max = 32))]
    pub arch: Option<String>,
    /// Seconds since the computer started
    #[schemars(range(min = 0))]
    pub uptime: f64,
    pub cpu: Cpu,
    pub memory: Memory,
    #[schemars(length(max = 16))]
    pub gpus: Vec<Gpu>,
    /// Where the GPU figures come from: nvidia-smi, ioreg, amdgpu, windows or demo; unavailable when that reader had nothing; null with no reader
    #[schemars(with = "Opt<Option<String>>", length(max = 32))]
    pub gpu: Option<String>,
    /// The last few minutes, oldest first; left out of a reading from another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 90))]
    pub history: Opt<Vec<Moment>>,
    pub hardware: Hardware,
}

// Another machine's latest reading, with its id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HostReading {
    #[serde(flatten)]
    pub reading: Reading,
    /// The machine's id in the network
    #[schemars(regex(pattern = r"^[a-z][a-z0-9-]{1,31}$"))]
    pub host: String,
}

/// What a computer running XenoNet is doing: its CPU, memory and GPUs now and lately, and what the parts are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct System {
    #[serde(flatten)]
    pub reading: Reading,
    /// With ?hosts=1: the latest reading each other machine in the network sent this console, its time moved to this clock. Only window hosts' consoles get them, while they watch, from machines that share their load
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 200))]
    pub hosts: Opt<Vec<HostReading>>,
}

impl Document for System {
    const ID: &'static str = "system/v1";
    const TITLE: &'static str = "A computer's load";
    const DESCRIPTION: &'static str = "What a computer running XenoNet is doing: its CPU, memory and GPUs now and lately, and what the parts are.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<System>];
