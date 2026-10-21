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

/// os.cpus(), os.totalmem(), os.freemem(), os.uptime(), as Node reads them
/// on macOS, Linux and Windows.
pub mod os {
    use super::Times;

    pub struct Cpus {
        pub model: Option<String>,
        pub speed: Option<f64>,
        pub times: Vec<Times>,
    }

    #[cfg(target_os = "macos")]
    #[expect(unsafe_code, reason = "sysctlbyname(3), which std doesn't wrap")]
    fn sysctl_bytes(name: &str) -> Option<Vec<u8>> {
        let c = std::ffi::CString::new(name).ok()?;
        let mut len: libc::size_t = 0;
        // SAFETY: a size query, then a read into a buffer of that size.
        unsafe {
            if libc::sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &raw mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
                return None;
            }
            let mut buf = vec![0u8; len];
            if libc::sysctlbyname(c.as_ptr(), buf.as_mut_ptr().cast(), &raw mut len, std::ptr::null_mut(), 0) != 0 {
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
    #[expect(unsafe_code, reason = "host_processor_info and vm_deallocate, the Mach calls for each CPU's ticks")]
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
            if libc::host_processor_info(libc::mach_host_self(), libc::PROCESSOR_CPU_LOAD_INFO, &raw mut count, &raw mut info, &raw mut info_count) == libc::KERN_SUCCESS {
                let ticks = libc::sysconf(libc::_SC_CLK_TCK).max(1) as f64;
                let k = 1000.0 / ticks;
                let all = std::slice::from_raw_parts(info, info_count as usize);
                for c in 0..count as usize {
                    let at = |s: i32| f64::from(all.get(c * libc::CPU_STATE_MAX as usize + s as usize).copied().unwrap_or(0) as u32) * k;
                    times.push(Times { user: at(libc::CPU_STATE_USER), nice: at(libc::CPU_STATE_NICE), sys: at(libc::CPU_STATE_SYSTEM), idle: at(libc::CPU_STATE_IDLE), irq: 0.0 });
                }
                libc::vm_deallocate(libc::mach_task_self(), info as libc::vm_address_t, (info_count as usize * size_of::<libc::integer_t>()) as libc::vm_size_t);
            }
        }
        Cpus { model, speed, times }
    }

    #[cfg(target_os = "macos")]
    pub fn totalmem() -> f64 {
        sysctl_u64("hw.memsize").unwrap_or(0) as f64
    }

    #[cfg(target_os = "macos")]
    #[expect(unsafe_code, reason = "host_statistics64, the Mach call for free memory")]
    pub fn freemem() -> f64 {
        // SAFETY: host_statistics64 fills the struct it's given, within the
        // count given.
        #[expect(deprecated, reason = "libc points its mach calls to the mach2 crate, a dependency for two calls")]
        unsafe {
            let mut info: libc::vm_statistics64 = std::mem::zeroed();
            let mut count = libc::HOST_VM_INFO64_COUNT;
            if libc::host_statistics64(libc::mach_host_self(), libc::HOST_VM_INFO64, (&raw mut info).cast(), &raw mut count) != libc::KERN_SUCCESS {
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
    #[expect(unsafe_code, reason = "sysconf(3), which std doesn't wrap")]
    pub fn cpus() -> Cpus {
        // SAFETY: sysconf takes a constant and reads nothing of ours.
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

    #[cfg(windows)]
    pub fn cpus() -> Cpus {
        Cpus { model: win::processor_name(), speed: win::processor_mhz().map(f64::from), times: win::processor_times().unwrap_or_else(idle_cores) }
    }

    #[cfg(windows)]
    pub fn totalmem() -> f64 {
        win::memory().map_or(0.0, |m| win::physical(&m).0)
    }

    #[cfg(windows)]
    pub fn freemem() -> f64 {
        win::memory().map_or(0.0, |m| win::physical(&m).1)
    }

    #[cfg(windows)]
    pub fn uptime() -> f64 {
        win::uptime(win::tick_count())
    }

    /// Windows, read as libuv reads it for Node's os module: each core's
    /// times from NtQuerySystemInformation (as uv_cpu_info does, since the
    /// panel shows each core), the processor's name and speed from the
    /// registry, memory from GlobalMemoryStatusEx and the uptime from
    /// GetTickCount64. What the calls fill in becomes the os module's
    /// numbers apart from the calls, so that's tested on every platform;
    /// the calls themselves only on Windows.
    #[cfg(any(windows, test))]
    pub mod win {
        use super::Times;

        /// SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: a core's times since
        /// boot, in 100 ns units. The kernel's time counts its idle time too.
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default)]
        pub struct ProcessorTimes {
            pub idle: i64,
            pub kernel: i64,
            pub user: i64,
            pub dpc: i64,
            pub interrupt: i64,
            pub interrupts: u32,
        }

        /// MEMORYSTATUSEX, in bytes.
        #[repr(C)]
        #[derive(Clone, Copy, Debug, Default)]
        pub struct MemoryStatus {
            pub length: u32,
            pub load: u32,
            pub total_phys: u64,
            pub avail_phys: u64,
            pub total_page_file: u64,
            pub avail_page_file: u64,
            pub total_virtual: u64,
            pub avail_virtual: u64,
            pub avail_extended_virtual: u64,
        }

        /// A core's times in milliseconds, as uv_cpu_info gives them: sys is
        /// the kernel's time less the idle time, irq the interrupts', and
        /// there's no nice.
        pub fn times(p: &ProcessorTimes) -> Times {
            let ms = |t: i64| (t / 10_000) as f64;
            Times { user: ms(p.user), nice: 0.0, sys: ms(p.kernel.saturating_sub(p.idle).max(0)), irq: ms(p.interrupt), idle: ms(p.idle) }
        }

        /// The cores a query filled in, by the bytes it says it returned.
        pub fn filled(cores: &[ProcessorTimes], returned: u32) -> &[ProcessorTimes] {
            let n = returned as usize / size_of::<ProcessorTimes>();
            &cores[..n.min(cores.len())]
        }

        /// os.totalmem() and os.freemem(), as libuv reads them: the
        /// physical memory, and how much of it is available.
        pub fn physical(m: &MemoryStatus) -> (f64, f64) {
            (m.total_phys as f64, m.avail_phys as f64)
        }

        /// os.uptime() in whole seconds, as on Linux, from GetTickCount64's
        /// milliseconds.
        pub fn uptime(ms: u64) -> f64 {
            (ms / 1000) as f64
        }

        /// A REG_SZ value's bytes as RegGetValueW writes them: UTF-16 (little
        /// endian, as Windows is), up to its NUL. None when it's empty.
        pub fn text(bytes: &[u8]) -> Option<String> {
            let wide: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).take_while(|c| *c != 0).collect();
            let s = String::from_utf16_lossy(&wide);
            (!s.is_empty()).then_some(s)
        }

        /// A REG_DWORD value's bytes as a number.
        pub fn dword(bytes: &[u8]) -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
        }

        /// Text as the Windows calls take it: UTF-16, ending in a NUL.
        pub fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        /// The first processor's key, where libuv reads its name and speed.
        #[cfg(windows)]
        const PROCESSOR: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

        /// Each core's times, or None when Windows won't say. The buffer
        /// starts with room for 64 cores, a processor group's most, and
        /// grows if Windows says it's too small.
        #[cfg(windows)]
        #[expect(unsafe_code, reason = "NtQuerySystemInformation, which std doesn't wrap")]
        pub fn processor_times() -> Option<Vec<Times>> {
            const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: u32 = 8;
            const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
            #[link(name = "ntdll")]
            extern "system" {
                fn NtQuerySystemInformation(class: u32, info: *mut std::ffi::c_void, length: u32, returned: *mut u32) -> i32;
            }
            let mut n = std::thread::available_parallelism().map_or(1, std::num::NonZero::get).max(64);
            for _ in 0..4 {
                let mut cores = vec![ProcessorTimes::default(); n];
                let length = u32::try_from(n * size_of::<ProcessorTimes>()).ok()?;
                let mut returned = 0u32;
                // SAFETY: `cores` is `length` bytes of ProcessorTimes, laid
                // out as SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION, which
                // Windows writes no more than; it and `returned` outlive the
                // call.
                let status = unsafe { NtQuerySystemInformation(SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION, cores.as_mut_ptr().cast(), length, &raw mut returned) };
                if status == STATUS_INFO_LENGTH_MISMATCH {
                    n = (returned as usize / size_of::<ProcessorTimes>()).max(n * 2);
                    continue;
                }
                // An NTSTATUS below zero is an error or a warning.
                let read: Vec<Times> = if status >= 0 { filled(&cores, returned).iter().map(times).collect() } else { Vec::new() };
                return (!read.is_empty()).then_some(read);
            }
            None
        }

        /// The processor's name, as the registry has it.
        #[cfg(windows)]
        pub fn processor_name() -> Option<String> {
            const RRF_RT_REG_SZ: u32 = 0x0000_0002;
            text(&processor_value("ProcessorNameString", RRF_RT_REG_SZ, 512)?)
        }

        /// The processor's speed in MHz, as the registry has it.
        #[cfg(windows)]
        pub fn processor_mhz() -> Option<u32> {
            const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
            dword(&processor_value("~MHz", RRF_RT_REG_DWORD, 4)?).filter(|mhz| *mhz != 0)
        }

        /// A value of the first processor's key, of the type `flags` lets
        /// through, as RegGetValueW writes it into `room` bytes; None when
        /// there's none, or it doesn't fit.
        #[cfg(windows)]
        #[expect(unsafe_code, reason = "RegGetValueW, which std doesn't wrap")]
        fn processor_value(name: &str, flags: u32, room: usize) -> Option<Vec<u8>> {
            // HKEY_LOCAL_MACHINE: (HKEY)(LONG)0x80000002, sign-extended.
            const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
            #[link(name = "advapi32")]
            extern "system" {
                fn RegGetValueW(key: isize, sub_key: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut std::ffi::c_void, bytes: *mut u32) -> i32;
            }
            let (key, value) = (wide(PROCESSOR), wide(name));
            let mut buf = vec![0u8; room];
            let mut bytes = u32::try_from(room).ok()?;
            // SAFETY: the key's and the value's names are NUL-terminated
            // UTF-16, and `buf` has room for `bytes`, which RegGetValueW
            // writes no more than (it fails rather than overrun); all of them
            // outlive the call.
            let status = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &raw mut bytes) };
            if status != 0 {
                return None;
            }
            buf.truncate(bytes as usize);
            Some(buf)
        }

        /// The memory, or None when Windows won't say.
        #[cfg(windows)]
        #[expect(unsafe_code, reason = "GlobalMemoryStatusEx, which std doesn't wrap")]
        pub fn memory() -> Option<MemoryStatus> {
            #[link(name = "kernel32")]
            extern "system" {
                fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
            }
            let mut m = MemoryStatus { length: size_of::<MemoryStatus>() as u32, ..MemoryStatus::default() };
            // SAFETY: `m` is a MEMORYSTATUSEX with its length set, as the
            // call asks, and outlives it.
            (unsafe { GlobalMemoryStatusEx(&raw mut m) } != 0).then_some(m)
        }

        /// Milliseconds since Windows started.
        #[cfg(windows)]
        #[expect(unsafe_code, reason = "GetTickCount64, which std doesn't wrap")]
        pub fn tick_count() -> u64 {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetTickCount64() -> u64;
            }
            // SAFETY: GetTickCount64 takes nothing and returns a number.
            unsafe { GetTickCount64() }
        }
    }

    // Where the cores' use can't be read (Windows when it won't say, and
    // any other system), the panel shows their count, idle.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn idle_cores() -> Vec<Times> {
        let n = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        vec![Times { user: 0.0, nice: 0.0, sys: 0.0, idle: 0.0, irq: 0.0 }; n]
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    pub fn cpus() -> Cpus {
        Cpus { model: None, speed: None, times: idle_cores() }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    pub fn totalmem() -> f64 {
        0.0
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    pub fn freemem() -> f64 {
        0.0
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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
        return Some(Gpu::Ioreg(Mutex::new(stats::Polled::default())));
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
        st: Arc::new(Mutex::new(Stream {
            windows,
            proc: stats::Streamed::default(),
            nvidia: stats::NvidiaCards::default(),
            counters: stats::WindowsReadings::default(),
            child: None,
        })),
    };
    if let Some(smi) = smi {
        return Some(stream("nvidia-smi", false, smi, stats::nvidia_args()));
    }
    if plat == "linux" {
        let base = "/sys/class/drm";
        let names: Vec<String> = std::fs::read_dir(base).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        let cards: Vec<String> =
            stats::drm_cards(&names).into_iter().map(|n| format!("{base}/{n}/device")).filter(|dev| std::fs::read_to_string(format!("{dev}/gpu_busy_percent")).is_ok()).collect();
        return (!cards.is_empty()).then(|| Gpu::Amd { cards, polled: Mutex::new(stats::Polled::default()) });
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
        Arc::new(System { core: Mutex::new(stats::Stats::default()), gpu: tokio::sync::OnceCell::new(), last: Mutex::new(None), idle: Mutex::new(None) })
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

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn the_os_module_reads_something_real() {
        let c = os::cpus();
        assert!(!c.times.is_empty());
        assert!(c.times.iter().any(|t| t.idle > 0.0 || t.user > 0.0), "the cores' times, not only their count");
        assert!(os::totalmem() > 0.0 && os::freemem() > 0.0 && os::freemem() <= os::totalmem());
        assert!(os::uptime() > 0.0);
        if cfg!(windows) {
            assert!(c.model.is_some() && c.speed.is_some_and(|mhz| mhz > 0.0), "{:?} {:?}", c.model, c.speed);
        }
    }

    #[test]
    fn windows_numbers_as_libuv_reads_them() {
        use os::win;
        assert_eq!((size_of::<win::ProcessorTimes>(), size_of::<win::MemoryStatus>()), (48, 64), "laid out as Windows' structs");
        // 100 ns units: the kernel's time counts its idle time, which
        // isn't sys; there's no nice.
        let core = win::ProcessorTimes { idle: 90_000_000, kernel: 120_000_000, user: 45_000_000, dpc: 1_000_000, interrupt: 2_500_000, interrupts: 7 };
        let t = win::times(&core);
        assert_eq!((t.user, t.nice, t.sys, t.irq, t.idle), (4500.0, 0.0, 3000.0, 250.0, 9000.0));
        // Whole milliseconds, as uv_cpu_info divides; never a negative sys.
        let odd = win::times(&win::ProcessorTimes { idle: 19_999, kernel: 9_999, user: 10_001, ..Default::default() });
        assert_eq!((odd.user, odd.sys, odd.idle), (1.0, 0.0, 1.0));
        // Only the cores the query filled in.
        let cores = [core; 4];
        assert_eq!(win::filled(&cores, 96).len(), 2);
        assert_eq!(win::filled(&cores, 48 * 9).len(), 4);
        assert_eq!(win::filled(&cores, 47).len(), 0);
        // The panel's arithmetic reads them as it reads macOS's and Linux's.
        let mut s = stats::Stats::default();
        s.set_prev(&[win::times(&core)]);
        // A quarter of a second busy (user) in a second, the rest idle.
        let later = win::ProcessorTimes { idle: 90_000_000 + 7_500_000, kernel: 120_000_000 + 7_500_000, user: 45_000_000 + 2_500_000, ..core };
        let cpu = s.cpu(Some("Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz"), Some(3192.0), 1, &[win::times(&later)]);
        assert_eq!((cpu["usage"].as_f64(), cpu["cores"][0].as_f64(), cpu["ghz"].as_f64()), (Some(0.25), Some(0.25), Some(3.2)));
        assert_eq!(cpu["model"], "Intel Core i7-8700");
        let m = win::MemoryStatus { total_phys: 17_179_869_184, avail_phys: 6_442_450_944, ..Default::default() };
        assert_eq!(win::physical(&m), (17_179_869_184.0, 6_442_450_944.0));
        assert_eq!((win::uptime(3_725_999), win::uptime(999)), (3725.0, 0.0));
    }

    #[test]
    fn windows_registry_values_as_regget_value_writes_them() {
        use os::win;
        let bytes = |s: &str| s.encode_utf16().chain([0, 0x41]).flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        assert_eq!(win::text(&bytes("AMD Ryzen 9 7950X 16-Core Processor  ")).as_deref(), Some("AMD Ryzen 9 7950X 16-Core Processor  "), "up to its NUL, as Node gives it");
        assert_eq!(win::text(&bytes("")), None);
        assert_eq!(win::text(&[0x49, 0x00, 0x6e]).as_deref(), Some("I"), "an odd byte at the end is left");
        assert_eq!(win::dword(&3192u32.to_le_bytes()), Some(3192));
        assert_eq!(win::dword(&[1, 2]), None);
        assert_eq!(win::wide("~MHz"), [0x7e, 0x4d, 0x48, 0x7a, 0]);
    }
}
