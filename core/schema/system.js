// system/v1: what a computer running XenoNet is doing, as GET /api/system
// answers the console's load panel (core/system-stats.js, src/system-panel.js):
// the CPU, memory and GPUs now, the last few minutes of each, and the parts
// they are (hardware). In a network, a machine's service sends its reading
// to the window hosts' consoles that watch its map, as the relay's `system`
// event (docs/design/network.md, section 7), without its history or its
// cores; GET /api/system?hosts=1 adds the latest from each machine.
import { define } from "./registry.js";
import { z, obj, ms, count } from "./common.js";
import { HOST_ID } from "./ids.js";

// The most a reading lists: samples in its history (three minutes at two
// seconds), cores, and GPUs.
export const SYSTEM_HISTORY = 90;
const MAX_CORES = 1024;
const MAX_GPUS = 16;

const share = z.number().min(0).max(1).describe("A share, 0 to 1");
const bytes = z.number().min(0).describe("Bytes");

const Gpu = obj({
  index: count,
  name: z.string().max(200),
  usage: share.nullable(),
  memUsed: bytes.nullable().describe("Its own memory in use; for a GPU that shares the computer's memory (Apple's), the memory it uses of that"),
  memTotal: bytes.nullable().describe("Its own memory; for a shared one, the computer's"),
  temp: z.number().nullable().describe("°C, where the reader has it (NVIDIA's)"),
  shared: z.boolean().describe("It shares the computer's memory, as Apple's GPUs do"),
});

const Hardware = obj({
  platform: z.string().max(32).describe("process.platform: darwin, linux, win32"),
  arch: z.string().max(32).nullable(),
  cpu: obj({ model: z.string().max(200).nullable(), threads: count.describe("Logical processors"), ghz: z.number().min(0).nullable() }),
  memory: obj({ total: bytes }),
  gpus: z.array(obj({ name: z.string().max(200), memTotal: bytes.nullable(), shared: z.boolean() })).max(MAX_GPUS),
}).describe("What the parts are, from what the readers read: no names, serials or OS version");

const Reading = obj({
  at: ms,
  platform: z.string().max(32),
  arch: z.string().max(32).nullable(),
  uptime: z.number().min(0).describe("Seconds since the computer started"),
  cpu: obj({
    model: z.string().max(200).nullable(),
    threads: count,
    ghz: z.number().min(0).nullable(),
    usage: share.describe("All of its threads, since the last reading"),
    cores: z.array(share).max(MAX_CORES).optional().describe("Each thread's use; left out of a reading from another machine"),
  }),
  memory: obj({ total: bytes, used: bytes }),
  gpus: z.array(Gpu).max(MAX_GPUS),
  gpu: z.string().max(32).nullable().describe("Where the GPU figures come from: nvidia-smi, ioreg, amdgpu, windows or demo; unavailable when that reader had nothing; null with no reader"),
  history: z
    .array(obj({ at: ms, cpu: share.nullable(), gpu: share.nullable(), mem: share.nullable() }))
    .max(SYSTEM_HISTORY)
    .optional()
    .describe("The last few minutes, oldest first; left out of a reading from another machine"),
  hardware: Hardware,
});

export const SystemReading = define(
  "system",
  1,
  Reading.extend({
    hosts: z
      .array(Reading.extend({ host: z.string().regex(HOST_ID).describe("The machine's id in the network") }))
      .max(200)
      .optional()
      .describe("With ?hosts=1: the latest reading each other machine in the network sent this console, its time moved to this clock. Only window hosts' consoles get them, while they watch, from machines that share their load"),
  }),
  {
    title: "A computer's load",
    description: "What a computer running XenoNet is doing: its CPU, memory and GPUs now and lately, and what the parts are.",
  },
);
