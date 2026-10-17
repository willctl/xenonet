//! core/system-stats.js: what the computer running XenoNet is doing, for
//! the console's system panel: the CPU (its model, how busy, each core),
//! memory, and the GPU (its name, how busy, its memory). Read only while a
//! window asks; none of it leaves the computer.
//!
//! The GPU comes from what the system has: NVIDIA's nvidia-smi left
//! running in its loop mode, a Mac's IOAccelerator statistics (ioreg), the
//! amdgpu driver's files in /sys on Linux, or Windows' own GPU counters in
//! a PowerShell loop. This has the readings' parsers, the CPU arithmetic,
//! and the state of each reader (a program kept running, a reading
//! polled); the twin starts and stops the programs and reads the files.

use crate::jsv::{raw, truthy};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;
use xenonet_base::json::number as num_v;
use xenonet_base::text::{js_number, js_string, js_trim};
use xenonet_base::units::js_round;

/// Samples kept for the traces: three minutes at 2 s.
pub const HISTORY: usize = 90;
/// A read this soon after the last one gets the same.
pub const FRESH_MS: f64 = 900.0;
pub const IDLE_MS: f64 = 60e3;
/// A GPU reading older than this isn't shown.
pub const STALE_MS: f64 = 15e3;
const MIB: f64 = 1024.0 * 1024.0;

// JS's \s, which Rust's differs from at U+0085 and U+FEFF.
const SPACE: &str = r"[\t\n\x0B\x0C\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";

pub fn frac(part: f64, whole: f64) -> f64 {
    if whole > 0.0 {
        (part / whole).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

// A figure as the JS's num() reads it: a finite number, or None.
fn num(s: Option<&str>) -> Option<f64> {
    let t = js_trim(s.unwrap_or(""));
    if t.is_empty() {
        return None;
    }
    Some(js_number(Some(&Value::String(t.into())))).filter(|n| n.is_finite())
}

fn opt(n: Option<f64>) -> Value {
    n.map_or(Value::Null, num_v)
}

/// "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz" → "Intel Core i7-9700K".
pub fn cpu_model(model: &str) -> Option<String> {
    static R: OnceLock<[Regex; 6]> = OnceLock::new();
    let r = R.get_or_init(|| {
        [
            Regex::new(r"(?i)\((?:R|TM|C)\)").unwrap(),
            Regex::new(r"(?-u:\b)CPU(?-u:\b)").unwrap(),
            Regex::new(&format!(r"(?i)@{SPACE}*[0-9.]+{SPACE}*GHz")).unwrap(),
            Regex::new(r"(?i)(?-u:\b)[0-9]+-Core Processor(?-u:\b)").unwrap(),
            Regex::new(r"(?i)(?-u:\b)Processor(?-u:\b)").unwrap(),
            Regex::new(&format!("{SPACE}+")).unwrap(),
        ]
    });
    let s = r[0].replace_all(model, "");
    let s = r[1].replace_all(&s, "");
    let s = r[2].replace(&s, "");
    let s = r[3].replace(&s, "");
    let s = r[4].replace(&s, "");
    let s = r[5].replace_all(&s, " ");
    let t = js_trim(&s);
    (!t.is_empty()).then(|| t.to_string())
}

/// What nvidia-smi is asked for, a line a GPU.
pub const NVIDIA_QUERY: &str = "index,name,utilization.gpu,memory.used,memory.total,temperature.gpu";

fn gpu(index: f64, name: &str, usage: Option<f64>, mem_used: Option<f64>, mem_total: Option<f64>, temp: Option<f64>, shared: bool) -> Value {
    json!({ "index": num_v(index), "name": name, "usage": opt(usage), "memUsed": opt(mem_used), "memTotal": opt(mem_total), "temp": opt(temp), "shared": shared })
}

/// One line of nvidia-smi's CSV ("0, NVIDIA RTX A4500, 41, 6000, 20470,
/// 58"). A figure the card doesn't report reads [N/A] or [Not Supported].
pub fn parse_nvidia(line: &str) -> Option<Value> {
    let parts: Vec<&str> = line.split(',').map(js_trim).collect();
    if parts.len() < 6 {
        return None;
    }
    let index = num(Some(parts[0]))?;
    let tail: Vec<Option<f64>> = parts[parts.len() - 4..].iter().map(|p| num(Some(p))).collect();
    let name = parts[1..parts.len() - 4].join(", ");
    if name.is_empty() {
        return None;
    }
    Some(gpu(index, &name, tail[0].map(|u| u / 100.0), tail[1].map(|u| u * MIB), tail[2].map(|t| t * MIB), tail[3], false))
}

/// ioreg's IOAccelerator: on Apple silicon, "model" names the chip and
/// PerformanceStatistics has "Device Utilization %" and "In use system
/// memory"; an AMD card in an Intel Mac has "vramUsedBytes" and
/// "VRAM,totalMB" instead.
pub fn parse_ioreg(text: &str, total_mem: Option<f64>) -> Option<Value> {
    if !text.contains("PerformanceStatistics") {
        return None;
    }
    let stat = |key: &str| -> Option<f64> {
        let re = Regex::new(&format!("\"{}\"=([0-9]+)", regex::escape(key))).ok()?;
        re.captures(text).map(|c| js_number(Some(&Value::String(c[1].to_string()))))
    };
    let usage = stat("Device Utilization %").or_else(|| stat("GPU Activity(%)"));
    let vram_used = stat("vramUsedBytes");
    let vram_total = stat("VRAM,totalMB");
    static NAME: OnceLock<[Regex; 3]> = OnceLock::new();
    let r = NAME.get_or_init(|| [Regex::new(r#""model" = "([^"]+)""#).unwrap(), Regex::new(r#""model" = <"([^"]+)">"#).unwrap(), Regex::new(r#""gpu-core-count" = ([0-9]+)"#).unwrap()]);
    let name = r[0].captures(text).or_else(|| r[1].captures(text)).map(|c| c[1].to_string());
    let cores = r[2].captures(text).map(|c| c[1].to_string());
    if let Some(total) = vram_total.filter(|t| *t != 0.0) {
        return Some(gpu(0.0, name.as_deref().unwrap_or("GPU"), usage.map(|u| u / 100.0), vram_used, Some(total * MIB), None, false));
    }
    let shown = match &name {
        Some(n) => format!("{n}{}", cores.map_or(String::new(), |c| format!(" · {c}-core GPU"))),
        None => "GPU".into(),
    };
    Some(gpu(0.0, &shown, usage.map(|u| u / 100.0), stat("In use system memory"), total_mem, None, true))
}

/// What Windows' counter loop said, a line at a time: the adapters
/// (largest memory first) and the latest reading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowsGpu {
    pub adapters: Vec<(String, f64)>,
    pub latest: Option<Value>,
}

/// What a line of the loop changed: the adapters, or the latest reading.
#[derive(Debug, PartialEq)]
pub enum WindowsLine {
    Adapters(Vec<(String, f64)>),
    Latest(Value),
    Nothing,
}

fn n_or0(v: Option<&Value>) -> f64 {
    let n = js_number(v);
    if n.is_nan() {
        0.0
    } else {
        n
    }
}

/// One line of the PowerShell loop (windows_gpu_script): "A " and the
/// display adapters with their memory, or "S " and, by adapter, the 3D
/// engines' use added up and the dedicated memory in use. Windows doesn't
/// say which adapter is which, so the one using the most dedicated memory
/// is taken for the one with the most.
pub fn parse_windows_gpu(line: &str, adapters: &[(String, f64)]) -> WindowsLine {
    let (kind, rest) = if line.starts_with("A ") || line.starts_with("S ") { (&line[..2], &line[2..]) } else { ("", line.get(2..).unwrap_or("")) };
    let Ok(body) = xenonet_base::json::parse(rest) else { return WindowsLine::Nothing };
    match kind {
        "A " => {
            let list: Vec<Value> = match body {
                Value::Array(a) => a,
                other => vec![other],
            };
            let mut out: Vec<(String, f64)> = list.iter().filter(|a| truthy(raw(Some(a), "name"))).map(|a| (js_string(&a["name"]), n_or0(raw(Some(a), "mem")))).collect();
            out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            WindowsLine::Adapters(out)
        }
        "S " => {
            let luids: Vec<Value> = match body {
                Value::Object(m) => m.into_iter().map(|(_, v)| v).filter(|v| v.is_object() || v.is_array()).collect(),
                Value::Array(a) => a.into_iter().filter(|v| v.is_object() || v.is_array()).collect(),
                _ => Vec::new(),
            };
            let Some(top) = luids.iter().max_by(|a, b| n_or0(raw(Some(a), "m")).partial_cmp(&n_or0(raw(Some(b), "m"))).unwrap_or(std::cmp::Ordering::Equal).then(std::cmp::Ordering::Greater)) else {
                return WindowsLine::Nothing;
            };
            let card = adapters.first();
            let mem_total = card.map(|c| c.1).filter(|m| *m != 0.0);
            WindowsLine::Latest(gpu(0.0, card.map_or("GPU", |c| c.0.as_str()), Some((n_or0(raw(Some(top), "u")) / 100.0).min(1.0)), Some(n_or0(raw(Some(top), "m"))), mem_total, None, false))
        }
        _ => WindowsLine::Nothing,
    }
}

// JS's `Number(x) | 0`.
fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    let m = n.trunc().rem_euclid(4294967296.0);
    (if m >= 2147483648.0 { m - 4294967296.0 } else { m }) as i32
}

/// The PowerShell loop, which ends itself once the service that started it
/// (`parent`) is gone, so a service that crashed leaves none behind.
pub fn windows_gpu_script(parent: f64) -> String {
    [
        "$ErrorActionPreference='SilentlyContinue'".to_string(),
        format!("$parent={}", to_int32(parent)),
        "$cls='HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}'".into(),
        "$a=@(Get-ChildItem $cls | ForEach-Object { $p=Get-ItemProperty $_.PSPath; if ($p.DriverDesc) { $m=$p.'HardwareInformation.qwMemorySize'; if (-not $m) { $m=$p.'HardwareInformation.MemorySize'; if ($m -is [byte[]]) { $m=[BitConverter]::ToUInt32($m,0) } }; [pscustomobject]@{name=$p.DriverDesc;mem=[uint64]$m} } })".into(),
        "'A '+(ConvertTo-Json -Compress -InputObject $a)".into(),
        "while (Get-Process -Id $parent) { $o=@{}; foreach ($x in (Get-Counter '\\GPU Engine(*engtype_3D)\\Utilization Percentage','\\GPU Adapter Memory(*)\\Dedicated Usage').CounterSamples) { if ($x.InstanceName -match 'luid_(0x[0-9a-f]+_0x[0-9a-f]+)') { $l=$Matches[1]; if (-not $o[$l]) { $o[$l]=@{u=0;m=0} }; if ($x.Path -like '*utilization percentage') { $o[$l].u+=$x.CookedValue } else { $o[$l].m+=$x.CookedValue } } }; 'S '+(ConvertTo-Json -Compress -InputObject $o); [Console]::Out.Flush(); Start-Sleep -Seconds 5 }".into(),
    ]
    .join("; ")
}

/// Where nvidia-smi is: on PATH, or where NVIDIA's driver puts it on
/// Windows. `exists` says whether a file is there.
pub fn find_nvidia_smi(platform: &str, path_var: &str, pathext: Option<&str>, system_root: Option<&str>, program_files: Option<&str>, exists: &mut dyn FnMut(&str) -> bool) -> Option<String> {
    if let Some(f) = xenonet_base::paths::on_path("nvidia-smi", path_var, pathext, platform == "win32", exists) {
        return Some(f);
    }
    if platform != "win32" {
        return None;
    }
    let w = crate::nodepath::Style::Win32;
    let root = system_root.unwrap_or("C:\\Windows");
    [w.join(&[root, "System32", "nvidia-smi.exe"]), w.join(&[program_files.unwrap_or("C:\\Program Files"), "NVIDIA Corporation", "NVSMI", "nvidia-smi.exe"])].into_iter().find(|f| exists(f))
}

/// What nvidia-smi is run with: its own loop, a line a GPU every 2 s.
pub fn nvidia_args() -> Vec<String> {
    vec![format!("--query-gpu={NVIDIA_QUERY}"), "--format=csv,noheader,nounits".into(), "-lms".into(), "2000".into()]
}

/// What PowerShell is run with for the counter loop.
pub fn windows_args(parent: f64) -> Vec<String> {
    vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), windows_gpu_script(parent)]
}

/// The windows reader's program: PowerShell, under SystemRoot.
pub fn windows_powershell(system_root: Option<&str>) -> String {
    crate::nodepath::Style::Win32.join(&[system_root.unwrap_or("C:\\Windows"), "System32", "WindowsPowerShell", "v1.0", "powershell.exe"])
}

const BACKOFF: [f64; 3] = [5e3, 20e3, 60e3];

/// A program kept running that prints a line a reading: started when
/// asked, stopped when idle, started again after it exits, a little later
/// each time, and given up on after three exits with nothing read.
#[derive(Debug, Default)]
pub struct Streamed {
    running: Option<u32>,
    generation: u32,
    buf: String,
    quiet: u32,
    read_since: bool,
    retry_at: f64,
    stopping: bool,
}

impl Streamed {
    pub fn dead(&self) -> bool {
        self.quiet >= 3
    }

    /// Whether to start the program now; if so, the generation its events
    /// carry.
    pub fn start(&mut self, now: f64) -> Option<u32> {
        if self.running.is_some() || self.quiet >= 3 || now < self.retry_at {
            return None;
        }
        self.stopping = false;
        self.read_since = false;
        self.generation += 1;
        self.running = Some(self.generation);
        Some(self.generation)
    }

    /// The program couldn't be started at all.
    pub fn spawn_failed(&mut self, now: f64) {
        self.running = None;
        self.quiet += 1;
        self.retry_at = now + BACKOFF[(self.quiet as usize - 1).min(2)];
    }

    /// What it printed: its whole lines, trimmed, without the blank ones.
    pub fn data(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut lines = Vec::new();
        while let Some(i) = self.buf.find('\n') {
            let line = js_trim(&self.buf[..i]).to_string();
            self.buf.drain(..=i);
            if line.is_empty() {
                continue;
            }
            self.read_since = true;
            self.quiet = 0;
            lines.push(line);
        }
        lines
    }

    /// The program of `generation` exited, or couldn't run.
    pub fn gone(&mut self, generation: u32, now: f64) {
        if self.running != Some(generation) {
            return;
        }
        self.running = None;
        self.buf.clear();
        if self.stopping {
            return;
        }
        if !self.read_since {
            self.quiet += 1;
        }
        self.retry_at = now + BACKOFF[(self.quiet.max(1) as usize - 1).min(2)];
    }

    /// Stops it; whether there was one to kill.
    pub fn stop(&mut self) -> bool {
        self.stopping = true;
        self.buf.clear();
        self.running.take().is_some()
    }
}

/// The GPUs nvidia-smi has reported, by index, with when.
#[derive(Debug, Default)]
pub struct NvidiaCards {
    cards: Vec<(f64, Value, f64)>,
}

impl NvidiaCards {
    pub fn line(&mut self, line: &str, now: f64) {
        if let Some(g) = parse_nvidia(line) {
            let index = g["index"].as_f64().unwrap_or(0.0);
            match self.cards.iter_mut().find(|c| c.0 == index) {
                Some(c) => {
                    c.1 = g;
                    c.2 = now;
                }
                None => self.cards.push((index, g, now)),
            }
        }
    }

    /// The fresh readings, by index.
    pub fn read(&self, now: f64) -> Vec<Value> {
        let mut fresh: Vec<&(f64, Value, f64)> = self.cards.iter().filter(|c| now - c.2 < STALE_MS).collect();
        fresh.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        fresh.into_iter().map(|c| c.1.clone()).collect()
    }
}

/// Windows' loop's readings, with when the latest came.
#[derive(Debug, Default)]
pub struct WindowsReadings {
    pub st: WindowsGpu,
    at: f64,
}

impl WindowsReadings {
    pub fn line(&mut self, line: &str, now: f64) {
        match parse_windows_gpu(line, &self.st.adapters) {
            WindowsLine::Adapters(a) => self.st.adapters = a,
            WindowsLine::Latest(l) => {
                self.st.latest = Some(l);
                self.at = now;
            }
            WindowsLine::Nothing => {}
        }
    }

    pub fn read(&self, now: f64) -> Vec<Value> {
        match &self.st.latest {
            Some(l) if now - self.at < STALE_MS * 2.0 => vec![l.clone()],
            _ => Vec::new(),
        }
    }
}

/// A reading run each time it's asked for, at most every couple of seconds.
#[derive(Debug, Default)]
pub struct Polled {
    pub last: Option<Value>,
    at: f64,
}

impl Polled {
    /// The last reading, while it's fresh.
    pub fn fresh(&self, now: f64) -> Option<Value> {
        (now - self.at < 1800.0).then(|| self.last.clone().unwrap_or(Value::Null))
    }

    pub fn set(&mut self, v: Value, now: f64) {
        self.last = Some(v);
        self.at = now;
    }
}

/// The /sys/class/drm entries that are cards.
pub fn drm_cards(names: &[String]) -> Vec<String> {
    names.iter().filter(|n| n.strip_prefix("card").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))).cloned().collect()
}

/// An amdgpu card from its files' contents (each trimmed, or None).
pub fn amd_card(index: usize, busy: Option<&str>, used: Option<&str>, total: Option<&str>, label: Option<&str>) -> Value {
    let name = label.filter(|l| !l.is_empty()).unwrap_or("AMD GPU");
    gpu(index as f64, name, num(busy).map(|b| b / 100.0), num(used), num(total), None, false)
}

/// A core's times as os.cpus() gives them.
#[derive(Clone, Copy, Debug)]
pub struct Times {
    pub user: f64,
    pub nice: f64,
    pub sys: f64,
    pub irq: f64,
    pub idle: f64,
}

#[derive(Clone, Copy, Debug)]
struct Load {
    busy: f64,
    total: f64,
}

fn loads(times: &[Times]) -> Vec<Load> {
    times.iter().map(|t| {
        let busy = t.user + t.nice + t.sys + t.irq;
        Load { busy, total: busy + t.idle }
    }).collect()
}

/// The CPU and memory side of the system panel: the cores' times at the
/// last reading, and the history of readings.
#[derive(Debug, Default)]
pub struct Stats {
    prev: Option<Vec<Load>>,
    history: Vec<Value>,
    last_at: f64,
    has_last: bool,
}

impl Stats {
    pub fn has_prev(&self) -> bool {
        self.prev.is_some()
    }

    /// Notes the cores' times before the first reading.
    pub fn set_prev(&mut self, times: &[Times]) {
        self.prev = Some(loads(times));
    }

    /// Whether a read now gets the last one.
    pub fn fresh(&self, now: f64) -> bool {
        self.has_last && now - self.last_at < FRESH_MS
    }

    /// The CPU: its model and speed (from the first core), each core's use
    /// since the last reading, and all of them.
    pub fn cpu(&mut self, model: Option<&str>, speed: Option<f64>, threads: usize, times: &[Times]) -> Value {
        let now = loads(times);
        let prev = self.prev.clone().unwrap_or_default();
        let cores: Vec<Value> = if now.len() == prev.len() { now.iter().zip(&prev).map(|(t, p)| num_v(frac(t.busy - p.busy, t.total - p.total))).collect() } else { now.iter().map(|_| json!(0)).collect() };
        let busy: f64 = now.iter().enumerate().fold(0.0, |s, (i, t)| s + t.busy - prev.get(i).map_or(t.busy, |p| p.busy));
        let total: f64 = now.iter().enumerate().fold(0.0, |s, (i, t)| s + t.total - prev.get(i).map_or(t.total, |p| p.total));
        self.prev = Some(now);
        let mhz = speed.filter(|s| *s != 0.0 && !s.is_nan());
        json!({ "model": model.and_then(cpu_model), "threads": threads, "ghz": mhz.map(|m| num_v(js_round(m / 100.0) / 10.0)), "usage": num_v(frac(busy, total)), "cores": cores })
    }

    /// The reading: the CPU, memory, the GPUs (None when the reader had
    /// nothing), which reader that was (None when there's none), and the
    /// history, which gains a sample at most every 1.5 s.
    #[allow(clippy::too_many_arguments)]
    pub fn reading(&mut self, at: f64, platform: &str, arch: Option<&str>, uptime: f64, cpu: Value, total: f64, free: f64, gpus: Option<Vec<Value>>, reader: Option<&str>) -> Value {
        let used = (total - free).max(0.0);
        let first = gpus.as_ref().and_then(|g| g.first()).cloned();
        let last_sample = self.history.last().and_then(|h| h["at"].as_f64());
        if last_sample.is_none_or(|l| at - l >= 1500.0) {
            self.history.push(json!({ "at": num_v(at), "cpu": cpu.get("usage").cloned().unwrap_or(Value::Null), "gpu": first.as_ref().and_then(|g| g.get("usage")).cloned().unwrap_or(Value::Null), "mem": num_v(frac(used, total)) }));
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        let gpu = match reader {
            None => Value::Null,
            Some(_) if gpus.is_none() => json!("unavailable"),
            Some(r) => json!(r),
        };
        self.last_at = at;
        self.has_last = true;
        json!({ "at": num_v(at), "platform": platform, "arch": arch, "uptime": num_v(js_round(uptime)), "cpu": cpu, "memory": { "total": num_v(total), "used": num_v(used) }, "gpus": gpus.unwrap_or_default(), "gpu": gpu, "history": self.history })
    }
}

fn wave(t: f64, period: f64, phase: f64) -> f64 {
    ((t / period) * std::f64::consts::PI * 2.0 + phase).sin()
}

fn sample(t: f64) -> (f64, f64, f64) {
    let cpu = 0.34 + 0.16 * wave(t, 41e3, 0.0) + 0.07 * wave(t, 9.7e3, 1.3);
    let gpu = 0.47 + 0.24 * wave(t, 57e3, 0.6) + 0.08 * wave(t, 6.1e3, 2.1);
    let mem = 0.61 + 0.04 * wave(t, 83e3, 0.2);
    (cpu, gpu, mem)
}

/// The demo's computer, made up: it breathes slowly, the same way each
/// time the clock comes round.
pub fn demo_reading(at: f64) -> Value {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let (cpu, gpu_u, mem) = sample(at);
    let step = 2000.0;
    let history: Vec<Value> = (0..HISTORY)
        .rev()
        .map(|i| {
            let t = (at / step).floor() * step - i as f64 * step;
            let (c, g, m) = sample(t);
            json!({ "at": num_v(t), "cpu": num_v(c), "gpu": num_v(g), "mem": num_v(m) })
        })
        .collect();
    let cores: Vec<Value> = (0..16).map(|i| num_v((cpu + 0.3 * wave(at, 3e3 + f64::from(i) * 410.0, f64::from(i))).clamp(0.02, 1.0))).collect();
    json!({
        "at": num_v(at),
        "platform": "linux",
        "arch": "x64",
        "uptime": 3 * 86400 + 4 * 3600 + 12 * 60,
        "cpu": { "model": "AMD Ryzen 7 7700X", "threads": 16, "ghz": 4.5, "usage": num_v(cpu), "cores": cores },
        "memory": { "total": num_v(64.0 * GB), "used": num_v(js_round(mem * 64.0 * GB)) },
        "gpus": [{ "index": 0, "name": "NVIDIA GeForce RTX 4080", "usage": num_v(gpu_u), "memUsed": num_v(js_round((7.4 + 1.2 * wave(at, 30e3, 0.0)) * GB)), "memTotal": num_v(16.0 * GB), "temp": num_v(js_round(56.0 + 9.0 * gpu_u)), "shared": false }],
        "gpu": "demo",
        "history": history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_cpu_names() {
        assert_eq!(cpu_model("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz").as_deref(), Some("Intel Core i7-9700K"));
        assert_eq!(cpu_model("AMD Ryzen 9 7950X 16-Core Processor            ").as_deref(), Some("AMD Ryzen 9 7950X"));
        assert_eq!(cpu_model(""), None);
    }

    #[test]
    fn nvidia_lines() {
        assert_eq!(parse_nvidia("1, Tesla T4, [N/A], 10, 15360, [Not Supported]").unwrap()["usage"], Value::Null);
        assert_eq!(parse_nvidia("2, Odd, Name, 5, 1, 2, 3").unwrap()["name"], "Odd, Name");
        assert_eq!(parse_nvidia("Failed to initialize NVML: Driver/library version mismatch"), None);
    }

    #[test]
    fn windows_counters() {
        let WindowsLine::Adapters(a) = parse_windows_gpu(r#"A [{"name":"Intel(R) UHD Graphics 770","mem":134217728},{"name":"NVIDIA RTX A4500","mem":21474836480}]"#, &[]) else { panic!() };
        assert_eq!(a[0].0, "NVIDIA RTX A4500");
        let WindowsLine::Latest(l) = parse_windows_gpu(r#"S {"0x00000000_0x00001a2b":{"u":2.5,"m":0},"0x00000000_0x00003c4d":{"u":64.3,"m":2147483648}}"#, &a) else { panic!() };
        assert_eq!(l["usage"], 0.643);
        assert!(windows_gpu_script(f64::NAN).contains("$parent=0;"));
        assert_eq!(to_int32(4294967297.0), 1);
    }

    #[test]
    fn a_program_that_wont_start_is_given_up_on() {
        let mut s = Streamed::default();
        let g = s.start(0.0).unwrap();
        assert_eq!(s.data("0, A, 1, 2, 3, 4\n0, A"), ["0, A, 1, 2, 3, 4"]);
        s.gone(g, 10.0);
        assert!(!s.dead());
        for i in 0..3 {
            let t = 100e3 * f64::from(i + 1);
            assert!(s.start(t).is_some());
            s.spawn_failed(t);
        }
        assert!(s.dead());
    }
}
