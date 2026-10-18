//! What the computer running XenoNet is doing, for the console's system
//! panel (core/system-stats.js, crates/xenonet-app/src/system_stats.rs).
//! The parsers, the CPU arithmetic, the history and each GPU reader's state
//! are the Rust's; this reads what Node's os module read (the cores'
//! times, memory, uptime), starts and stops the programs the GPU readers
//! keep running, and keeps the idle timer. The demo's computer is made up.

use crate::fx::Locked;
use crate::fx::{self, now_ms, platform, RunOptions};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use xenonet_app::system_stats::{self as stats, Times};
use xenonet_service::cores::{SystemStats, R};

/// os.cpus(), os.totalmem(), os.freemem(), os.uptime(), as Node reads them.
pub mod os {
    use super::Times;

    pub struct Cpus {
        pub model: Option<String>,
        pub speed: Option<f64>,
        pub times: Vec<Times>,
    }

    #[cfg(target_os = "macos")]
    fn sysctl_bytes(name: &str) -> Option<Vec<u8>> {
        let c = std::ffi::CString::new(name).ok()?;
        let mut len: libc::size_t = 0;
        // SAFETY: a size query, then a read into a buffer of that size.
        unsafe {
            if libc::sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            if libc::sysctlbyname(c.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
            buf.truncate(len);
            Some(buf)
        }
    }

    #[cfg(target_os = "macos")]
    fn sysctl_u64(name: &str) -> Option<u64> {
        let b = sysctl_bytes(name)?;
        match b.len() {
            8 => Some(u64::from_ne_bytes(b[..8].try_into().ok()?)),
            4 => Some(u64::from(u32::from_ne_bytes(b[..4].try_into().ok()?))),
            _ => None,
        }
    }

    #[cfg(target_os = "macos")]
    pub fn cpus() -> Cpus {
        let model = sysctl_bytes("machdep.cpu.brand_string").map(|b| String::from_utf8_lossy(&b).trim_end_matches('\0').to_string());
        let speed = sysctl_u64("hw.cpufrequency").map(|hz| (hz / 1_000_000) as f64);
        let mut times = Vec::new();
        // SAFETY: host_processor_info hands back an array it allocated,
        // read within its count and given back with vm_deallocate.
        #[expect(deprecated, reason = "libc points its mach calls to the mach2 crate, a dependency for two calls")]
        unsafe {
            let mut count: libc::natural_t = 0;
            let mut info: libc::processor_info_array_t = std::ptr::null_mut();
            let mut info_count: libc::mach_msg_type_number_t = 0;
            if libc::host_processor_info(libc::mach_host_self(), libc::PROCESSOR_CPU_LOAD_INFO, &mut count, &mut info, &mut info_count) == libc::KERN_SUCCESS {
                let ticks = libc::sysconf(libc::_SC_CLK_TCK).max(1) as f64;
                let k = 1000.0 / ticks;
                let all = std::slice::from_raw_parts(info, info_count as usize);
                for c in 0..count as usize {
                    let at = |s: i32| f64::from(all.get(c * libc::CPU_STATE_MAX as usize + s as usize).copied().unwrap_or(0) as u32) * k;
                    times.push(Times { user: at(libc::CPU_STATE_USER), nice: at(libc::CPU_STATE_NICE), sys: at(libc::CPU_STATE_SYSTEM), idle: at(libc::CPU_STATE_IDLE), irq: 0.0 });
                }
                libc::vm_deallocate(libc::mach_task_self(), info as libc::vm_address_t, (info_count as usize * std::mem::size_of::<libc::integer_t>()) as libc::vm_size_t);
            }
        }
        Cpus { model, speed, times }
    }

    #[cfg(target_os = "macos")]
    pub fn totalmem() -> f64 {
        sysctl_u64("hw.memsize").unwrap_or(0) as f64
    }

    #[cfg(target_os = "macos")]
    pub fn freemem() -> f64 {
        // SAFETY: host_statistics64 fills the struct it's given, within the
        // count given.
        #[expect(deprecated, reason = "libc points its mach calls to the mach2 crate, a dependency for two calls")]
        unsafe {
            let mut info: libc::vm_statistics64 = std::mem::zeroed();
            let mut count = libc::HOST_VM_INFO64_COUNT;
            if libc::host_statistics64(libc::mach_host_self(), libc::HOST_VM_INFO64, (&mut info as *mut libc::vm_statistics64).cast(), &mut count) != libc::KERN_SUCCESS {
                return 0.0;
            }
            f64::from(info.free_count) * libc::sysconf(libc::_SC_PAGESIZE) as f64
        }
    }

    #[cfg(target_os = "macos")]
    pub fn uptime() -> f64 {
        let Some(b) = sysctl_bytes("kern.boottime") else { return 0.0 };
        if b.len() < 8 {
            return 0.0;
        }
        let secs = i64::from_ne_bytes(b[..8].try_into().unwrap_or([0; 8])) as f64;
        (crate::fx::now_ms() / 1000.0 - secs).max(0.0).floor()
    }

    #[cfg(target_os = "linux")]
    pub fn cpus() -> Cpus {
        let k = 1000.0 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
        let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
        let times = stat
            .lines()
            .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
            .map(|l| {
                let n: Vec<f64> = l.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(0.0)).collect();
                let at = |i: usize| n.get(i).copied().unwrap_or(0.0) * k;
                Times { user: at(0), nice: at(1), sys: at(2), idle: at(3), irq: at(5) }
            })
            .collect();
        let info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let field = |name: &str| info.lines().find(|l| l.starts_with(name)).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim().to_string());
        Cpus { model: field("model name"), speed: field("cpu MHz").and_then(|s| s.parse::<f64>().ok()).map(f64::floor), times }
    }

    #[cfg(target_os = "linux")]
    fn meminfo(name: &str) -> Option<f64> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = text.lines().find(|l| l.starts_with(name))?;
        line.split_whitespace().nth(1)?.parse::<f64>().ok().map(|kb| kb * 1024.0)
    }

    #[cfg(target_os = "linux")]
    pub fn totalmem() -> f64 {
        meminfo("MemTotal:").unwrap_or(0.0)
    }

    #[cfg(target_os = "linux")]
    pub fn freemem() -> f64 {
        meminfo("MemAvailable:").or_else(|| meminfo("MemFree:")).unwrap_or(0.0)
    }

    #[cfg(target_os = "linux")]
    pub fn uptime() -> f64 {
        std::fs::read_to_string("/proc/uptime").ok().and_then(|t| t.split_whitespace().next().and_then(|s| s.parse::<f64>().ok())).map_or(0.0, f64::floor)
    }

    // Elsewhere (Windows among them) the panel shows what it can read:
    // the cores' count and nothing of their use.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn cpus() -> Cpus {
        let n = std::thread::available_parallelism().map_or(1, |n| n.get());
        Cpus { model: None, speed: None, times: vec![Times { user: 0.0, nice: 0.0, sys: 0.0, idle: 0.0, irq: 0.0 }; n] }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn totalmem() -> f64 {
        0.0
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn freemem() -> f64 {
        0.0
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn uptime() -> f64 {
        0.0
    }

    /// os.arch().
    pub fn arch() -> &'static str {
        match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            "x86" => "ia32",
            other => other,
        }
    }
}

// A program kept running that prints a line a reading (nvidia-smi, or
// Windows' counter loop); when to start it again, and when to give up, is
// the Rust's.
struct Stream {
    windows: bool,
    proc: stats::Streamed,
    nvidia: stats::NvidiaCards,
    counters: stats::WindowsReadings,
    child: Option<tokio::sync::oneshot::Sender<()>>,
}

enum Gpu {
    Stream { source: &'static str, file: String, args: Vec<String>, st: Arc<Mutex<Stream>> },
    Ioreg(Mutex<stats::Polled>),
    Amd { cards: Vec<String>, polled: Mutex<stats::Polled> },
}

impl Gpu {
    fn source(&self) -> &'static str {
        match self {
            Gpu::Stream { source, .. } => source,
            Gpu::Ioreg(_) => "ioreg",
            Gpu::Amd { .. } => "amdgpu",
        }
    }

    fn start(&self) {
        let Gpu::Stream { file, args, st, .. } = self else { return };
        let Some(generation) = st.locked().proc.start(now_ms()) else { return };
        let mut cmd = tokio::process::Command::new(file);
        cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let Ok(mut child) = cmd.spawn() else {
            st.locked().proc.spawn_failed(now_ms());
            return;
        };
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        st.locked().child = Some(stop_tx);
        let st = st.clone();
        let mut out = child.stdout.take();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            loop {
                let read = async {
                    match out.as_mut() {
                        Some(o) => o.read(&mut buf).await.unwrap_or(0),
                        None => 0,
                    }
                };
                tokio::select! {
                    n = read => {
                        if n == 0 { break; }
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        let mut s = st.locked();
                        let now = now_ms();
                        for line in s.proc.data(&chunk) {
                            if s.windows { s.counters.line(&line, now) } else { s.nvidia.line(&line, now) }
                        }
                    }
                    _ = &mut stop_rx => {
                        let _ = child.kill().await;
                        return;
                    }
                }
            }
            let _ = child.wait().await;
            st.locked().proc.gone(generation, now_ms());
        });
    }

    fn stop(&self) {
        if let Gpu::Stream { st, .. } = self {
            let mut s = st.locked();
            let child = s.child.take();
            if s.proc.stop() {
                if let Some(c) = child {
                    let _ = c.send(());
                }
            }
        }
    }

    async fn read(&self) -> Option<Vec<Value>> {
        match self {
            Gpu::Stream { st, .. } => {
                self.start();
                let s = st.locked();
                if s.proc.dead() {
                    return None;
                }
                Some(if s.windows { s.counters.read(now_ms()) } else { s.nvidia.read(now_ms()) })
            }
            Gpu::Ioreg(p) => {
                if let Some(hit) = p.locked().fresh(now_ms()) {
                    return hit.as_array().cloned();
                }
                let r = fx::exec_file("/usr/sbin/ioreg", &["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"].map(String::from), 3000, RunOptions::default()).await;
                let g = if r.ok() { stats::parse_ioreg(&r.stdout, Some(os::totalmem())) } else { None };
                let v = Value::Array(g.into_iter().collect());
                p.locked().set(v.clone(), now_ms());
                v.as_array().cloned()
            }
            Gpu::Amd { cards, polled } => {
                if let Some(hit) = polled.locked().fresh(now_ms()) {
                    return hit.as_array().cloned();
                }
                let read = |dev: &str, f: &str| std::fs::read_to_string(format!("{dev}/{f}")).ok().map(|s| s.trim().to_string());
                let v: Vec<Value> = cards
                    .iter()
                    .enumerate()
                    .map(|(i, dev)| {
                        stats::amd_card(
                            i,
                            read(dev, "gpu_busy_percent").as_deref(),
                            read(dev, "mem_info_vram_used").as_deref(),
                            read(dev, "mem_info_vram_total").as_deref(),
                            read(dev, "product_name").as_deref(),
                        )
                    })
                    .collect();
                polled.locked().set(Value::Array(v.clone()), now_ms());
                Some(v)
            }
        }
    }
}

fn pick() -> Option<Gpu> {
    let plat = platform();
    if plat == "darwin" {
        return Some(Gpu::Ioreg(Mutex::new(Default::default())));
    }
    let env = |k: &str| std::env::var(k).ok();
    let smi = stats::find_nvidia_smi(
        plat,
        &env("PATH").unwrap_or_default(),
        env("PATHEXT").as_deref(),
        env("SystemRoot").or_else(|| env("SYSTEMROOT")).as_deref(),
        env("ProgramFiles").as_deref(),
        &mut |f| std::path::Path::new(f).exists(),
    );
    let stream = |source: &'static str, windows: bool, file: String, args: Vec<String>| Gpu::Stream {
        source,
        file,
        args,
        st: Arc::new(Mutex::new(Stream { windows, proc: Default::default(), nvidia: Default::default(), counters: Default::default(), child: None })),
    };
    if let Some(smi) = smi {
        return Some(stream("nvidia-smi", false, smi, stats::nvidia_args()));
    }
    if plat == "linux" {
        let base = "/sys/class/drm";
        let names: Vec<String> = std::fs::read_dir(base).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        let cards: Vec<String> =
            stats::drm_cards(&names).into_iter().map(|n| format!("{base}/{n}/device")).filter(|dev| std::fs::read_to_string(format!("{dev}/gpu_busy_percent")).is_ok()).collect();
        return (!cards.is_empty()).then(|| Gpu::Amd { cards, polled: Mutex::new(Default::default()) });
    }
    if plat == "win32" {
        let file = stats::windows_powershell(env("SystemRoot").or_else(|| env("SYSTEMROOT")).as_deref());
        return Some(stream("windows", true, file, stats::windows_args(f64::from(std::process::id()))));
    }
    None
}

pub struct System {
    core: Mutex<stats::Stats>,
    // None until picked; Some(None) when there's nothing to read.
    gpu: tokio::sync::OnceCell<Option<Gpu>>,
    last: Mutex<Option<Value>>,
    idle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl System {
    pub fn new() -> Arc<Self> {
        Arc::new(System { core: Mutex::new(Default::default()), gpu: tokio::sync::OnceCell::new(), last: Mutex::new(None), idle: Mutex::new(None) })
    }

    async fn cpu(&self) -> Value {
        if !self.core.locked().has_prev() {
            self.core.locked().set_prev(&os::cpus().times);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let c = os::cpus();
        self.core.locked().cpu(c.model.as_deref(), c.speed, c.times.len(), &c.times)
    }
}

/// The readers go to sleep a minute after the last window's look.
pub struct SystemPanel(pub Arc<System>);

#[async_trait]
impl SystemStats for SystemPanel {
    async fn read(&self) -> R<Value> {
        let me = self.0.clone();
        {
            let mut idle = me.idle.locked();
            if let Some(t) = idle.take() {
                t.abort();
            }
            let sleepy = me.clone();
            *idle = Some(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(stats::IDLE_MS as u64)).await;
                if let Some(Some(g)) = sleepy.gpu.get() {
                    g.stop();
                }
            }));
        }
        if let Some(last) = me.last.locked().clone().filter(|_| me.core.locked().fresh(now_ms())) {
            return Ok(last);
        }
        let gpu = me.gpu.get_or_init(|| async { pick() }).await;
        let (c, g) = tokio::join!(me.cpu(), async {
            match gpu {
                Some(g) => g.read().await,
                None => None,
            }
        });
        let reading = me.core.locked().reading(now_ms(), platform(), Some(os::arch()), os::uptime(), c, os::totalmem(), os::freemem(), g, gpu.as_ref().map(Gpu::source));
        *me.last.locked() = Some(reading.clone());
        Ok(reading)
    }

    fn stop(&self) {
        if let Some(t) = self.0.idle.locked().take() {
            t.abort();
        }
        if let Some(Some(g)) = self.0.gpu.get() {
            g.stop();
        }
    }
}

/// The demo's computer: it breathes slowly, the same way each time the
/// clock comes round.
pub struct DemoSystem;

#[async_trait]
impl SystemStats for DemoSystem {
    async fn read(&self) -> R<Value> {
        Ok(stats::demo_reading(now_ms()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn this_computer_as_the_panel_reads_it() {
        let p = SystemPanel(System::new());
        let r = p.read().await.unwrap();
        assert!(r["cpu"]["threads"].as_u64().unwrap() >= 1);
        assert!(r["memory"]["total"].as_f64().unwrap() >= 0.0);
        assert_eq!(r["history"].as_array().unwrap().len(), 1);
        assert_eq!(p.read().await.unwrap()["at"], r["at"], "a fresh reading is the last one");
        p.stop();
        assert!(DemoSystem.read().await.unwrap()["cpu"]["cores"].as_array().unwrap().len() > 1);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_os_module_reads_something_real() {
        let c = os::cpus();
        assert!(!c.times.is_empty());
        assert!(os::totalmem() > 0.0 && os::freemem() <= os::totalmem());
        assert!(os::uptime() > 0.0);
    }
}
