//! Live host statistics like EasyPanel shows them: CPU, memory, network and
//! disk, sampled every [`INTERVAL`] and kept for the last [`HISTORY`]
//! samples in memory only (nothing is stored in the database).
//!
//! Where the numbers come from:
//! - The panel in a container (production) cannot see the host through its
//!   own `/proc`, so the installer mounts the host's `/proc` at `HOST_PROC`
//!   (`/host/proc`) and its `/` at `HOST_ROOT` (`/host/root`), both read-only;
//!   [`ProcfsProbe`] reads those.
//! - Otherwise (development, a panel running directly on the host)
//!   [`SysinfoProbe`] reads the machine it runs on.
//!
//! The sampler starts on first use ([`SystemStats::subscribe`],
//! [`SystemStats::history`]) because providers also boot for console
//! commands and tests. Once the framework's scheduler lands (RustCMS v0.4)
//! it starts with `serve` instead, so the first viewer already has history.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::broadcast;

pub const INTERVAL: Duration = Duration::from_secs(2);
/// About five minutes at [`INTERVAL`].
pub const HISTORY: usize = 150;

/// One point of the dashboard charts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HostSample {
    /// Unix milliseconds.
    pub at: i64,
    pub cpu_percent: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    /// Bytes per second since the previous sample.
    pub net_rx_rate: u64,
    pub net_tx_rate: u64,
    pub disk_used: u64,
    pub disk_total: u64,
}

/// What does not change between samples.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HostInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub architecture: String,
    pub cpus: usize,
    pub uptime_secs: u64,
}

/// A raw reading: network counters are totals, turned into rates by
/// [`sample`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    pub cpu_percent: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub net_rx: u64,
    pub net_tx: u64,
    pub disk_used: u64,
    pub disk_total: u64,
}

/// Reads the host. CPU usage needs two readings; the first one reports 0.
pub trait Probe: Send {
    fn read(&mut self) -> Reading;
    fn info(&self) -> HostInfo;
}

/// The sample between two readings `elapsed` apart.
pub fn sample(prev: Option<&Reading>, cur: &Reading, elapsed: Duration, at: i64) -> HostSample {
    let secs = elapsed.as_secs_f64();
    let rate = |now: u64, before: Option<u64>| match before {
        Some(before) if secs > 0.0 => (now.saturating_sub(before) as f64 / secs).round() as u64,
        _ => 0,
    };
    HostSample {
        at,
        cpu_percent: cur.cpu_percent.clamp(0.0, 100.0),
        mem_used: cur.mem_used,
        mem_total: cur.mem_total,
        swap_used: cur.swap_used,
        swap_total: cur.swap_total,
        net_rx_rate: rate(cur.net_rx, prev.map(|p| p.net_rx)),
        net_tx_rate: rate(cur.net_tx, prev.map(|p| p.net_tx)),
        disk_used: cur.disk_used,
        disk_total: cur.disk_total,
    }
}

/// The service the app binds (`app.service::<SystemStats>()`).
#[derive(Clone)]
pub struct SystemStats {
    inner: Arc<Inner>,
}

struct Inner {
    probe: Mutex<Box<dyn Probe>>,
    state: Mutex<State>,
    live: broadcast::Sender<HostSample>,
    started: Once,
}

#[derive(Default)]
struct State {
    history: VecDeque<HostSample>,
    last: Option<(Instant, Reading)>,
}

impl SystemStats {
    /// [`ProcfsProbe`] when `HOST_PROC` is set, else [`SysinfoProbe`].
    pub fn from_env() -> Self {
        match std::env::var_os("HOST_PROC") {
            Some(proc_root) => {
                let host_root =
                    std::env::var_os("HOST_ROOT").map_or_else(|| PathBuf::from("/"), PathBuf::from);
                Self::with_probe(ProcfsProbe::new(proc_root, host_root))
            }
            None => Self::with_probe(SysinfoProbe::new()),
        }
    }

    pub fn with_probe(probe: impl Probe + 'static) -> Self {
        let (live, _) = broadcast::channel(16);
        Self {
            inner: Arc::new(Inner {
                probe: Mutex::new(Box::new(probe)),
                state: Mutex::new(State::default()),
                live,
                started: Once::new(),
            }),
        }
    }

    pub fn info(&self) -> HostInfo {
        lock(&self.inner.probe).info()
    }

    /// The samples kept, oldest first.
    pub fn history(&self) -> Vec<HostSample> {
        self.ensure_started();
        lock(&self.inner.state).history.iter().cloned().collect()
    }

    /// Every new sample.
    pub fn subscribe(&self) -> broadcast::Receiver<HostSample> {
        self.ensure_started();
        self.inner.live.subscribe()
    }

    /// Reads the host once, records and publishes the sample. Blocking (it
    /// reads files): the sampler calls it on the blocking pool.
    pub fn sample_now(&self) -> HostSample {
        Self::tick(&self.inner)
    }

    fn tick(inner: &Inner) -> HostSample {
        let reading = lock(&inner.probe).read();
        let now = Instant::now();
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let mut state = lock(&inner.state);
        let (prev, elapsed) = match &state.last {
            Some((when, prev)) => (Some(prev), now.duration_since(*when)),
            None => (None, Duration::ZERO),
        };
        let point = sample(prev, &reading, elapsed, at);
        state.last = Some((now, reading));
        if state.history.len() == HISTORY {
            state.history.pop_front();
        }
        state.history.push_back(point.clone());
        drop(state);
        // No receivers is fine: nobody is watching.
        let _ = inner.live.send(point.clone());
        point
    }

    fn ensure_started(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak = Arc::downgrade(&self.inner);
        self.inner.started.call_once(|| {
            runtime.spawn(run(weak));
        });
    }
}

/// Samples until the service is dropped.
async fn run(inner: Weak<Inner>) {
    let mut ticker = tokio::time::interval(INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let Some(strong) = inner.upgrade() else {
            return;
        };
        if tokio::task::spawn_blocking(move || SystemStats::tick(&strong))
            .await
            .is_err()
        {
            tracing::warn!("host sampling panicked");
        }
    }
}

fn lock<T: ?Sized>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Interfaces whose traffic is not the host's own: loopback and Docker's
/// bridges and container ends (their traffic also crosses a real interface).
fn counts_as_host_interface(name: &str) -> bool {
    !(name == "lo"
        || ["veth", "docker", "br-"]
            .iter()
            .any(|prefix| name.starts_with(prefix)))
}

/// Used and total bytes of the filesystem holding `path`, as `df` counts them.
fn disk_usage(path: &Path) -> (u64, u64) {
    match rustix::fs::statvfs(path) {
        Ok(fs) => {
            let total = fs.f_blocks.saturating_mul(fs.f_frsize);
            let free = fs.f_bfree.saturating_mul(fs.f_frsize);
            (total.saturating_sub(free), total)
        }
        Err(_) => (0, 0),
    }
}

/// Reads the machine the panel runs on.
pub struct SysinfoProbe {
    system: sysinfo::System,
    networks: sysinfo::Networks,
}

impl SysinfoProbe {
    pub fn new() -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_cpu_usage();
        Self {
            system,
            networks: sysinfo::Networks::new_with_refreshed_list(),
        }
    }
}

impl Default for SysinfoProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe for SysinfoProbe {
    fn read(&mut self) -> Reading {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.networks.refresh(true);
        let (net_rx, net_tx) = self
            .networks
            .iter()
            .filter(|(name, _)| counts_as_host_interface(name))
            .fold((0, 0), |(rx, tx), (_, data)| {
                (rx + data.total_received(), tx + data.total_transmitted())
            });
        let (disk_used, disk_total) = disk_usage(Path::new("/"));
        Reading {
            cpu_percent: f64::from(self.system.global_cpu_usage()),
            mem_used: self.system.used_memory(),
            mem_total: self.system.total_memory(),
            swap_used: self.system.used_swap(),
            swap_total: self.system.total_swap(),
            net_rx,
            net_tx,
            disk_used,
            disk_total,
        }
    }

    fn info(&self) -> HostInfo {
        HostInfo {
            hostname: sysinfo::System::host_name().unwrap_or_default(),
            os: sysinfo::System::long_os_version().unwrap_or_default(),
            kernel: sysinfo::System::kernel_version().unwrap_or_default(),
            architecture: sysinfo::System::cpu_arch(),
            cpus: self.system.cpus().len(),
            uptime_secs: sysinfo::System::uptime(),
        }
    }
}

/// Reads the host's `/proc` and `/` mounted into the panel's container.
pub struct ProcfsProbe {
    proc_root: PathBuf,
    host_root: PathBuf,
    /// (busy, total) jiffies of the previous reading.
    last_cpu: Option<(u64, u64)>,
}

impl ProcfsProbe {
    pub fn new(proc_root: impl Into<PathBuf>, host_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            host_root: host_root.into(),
            last_cpu: None,
        }
    }

    fn file(&self, relative: &str) -> String {
        fs::read_to_string(self.proc_root.join(relative)).unwrap_or_default()
    }
}

impl Probe for ProcfsProbe {
    fn read(&mut self) -> Reading {
        let cpu = parse_cpu_jiffies(&self.file("stat"));
        let cpu_percent = match (self.last_cpu, cpu) {
            (Some((busy0, total0)), Some((busy1, total1))) if total1 > total0 => {
                busy1.saturating_sub(busy0) as f64 / (total1 - total0) as f64 * 100.0
            }
            _ => 0.0,
        };
        self.last_cpu = cpu.or(self.last_cpu);
        let mem = parse_meminfo(&self.file("meminfo"));
        // `net/dev` is per network namespace; the host's is that of its PID 1.
        let (net_rx, net_tx) = parse_net_dev(&self.file("1/net/dev"));
        let (disk_used, disk_total) = disk_usage(&self.host_root);
        Reading {
            cpu_percent,
            mem_used: mem.total.saturating_sub(mem.available),
            mem_total: mem.total,
            swap_used: mem.swap_total.saturating_sub(mem.swap_free),
            swap_total: mem.swap_total,
            net_rx,
            net_tx,
            disk_used,
            disk_total,
        }
    }

    fn info(&self) -> HostInfo {
        let os_release =
            fs::read_to_string(self.host_root.join("etc/os-release")).unwrap_or_default();
        HostInfo {
            hostname: self.file("sys/kernel/hostname").trim().to_owned(),
            os: parse_pretty_name(&os_release).unwrap_or_else(|| "Linux".to_owned()),
            kernel: self.file("sys/kernel/osrelease").trim().to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            cpus: self
                .file("stat")
                .lines()
                .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
                .count(),
            uptime_secs: self
                .file("uptime")
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .map_or(0, |secs| secs as u64),
        }
    }
}

/// (busy, total) jiffies from the aggregate `cpu` line of `/proc/stat`.
fn parse_cpu_jiffies(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    // user nice system idle iowait irq softirq steal (guest time is already in user).
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|f| f.parse().ok())
        .collect();
    if fields.len() < 4 {
        return None;
    }
    let total: u64 = fields.iter().sum();
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

#[derive(Debug, Default, PartialEq)]
struct MemInfo {
    total: u64,
    available: u64,
    swap_total: u64,
    swap_free: u64,
}

/// Bytes from `/proc/meminfo` (which counts in kB).
fn parse_meminfo(meminfo: &str) -> MemInfo {
    let mut mem = MemInfo::default();
    for line in meminfo.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = rest
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        let bytes = kb * 1024;
        match key {
            "MemTotal" => mem.total = bytes,
            "MemAvailable" => mem.available = bytes,
            "SwapTotal" => mem.swap_total = bytes,
            "SwapFree" => mem.swap_free = bytes,
            _ => {}
        }
    }
    mem
}

/// Received and transmitted bytes of the host's interfaces in `/proc/net/dev`.
fn parse_net_dev(net_dev: &str) -> (u64, u64) {
    net_dev
        .lines()
        .skip(2)
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            if !counts_as_host_interface(name.trim()) {
                return None;
            }
            let fields: Vec<u64> = counters
                .split_whitespace()
                .filter_map(|f| f.parse().ok())
                .collect();
            Some((*fields.first()?, *fields.get(8)?))
        })
        .fold((0, 0), |(rx, tx), (r, t)| (rx + r, tx + t))
}

fn parse_pretty_name(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat() {
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\n";
        assert_eq!(parse_cpu_jiffies(stat), Some((150, 1000)));
        assert_eq!(parse_cpu_jiffies("intr 1 2"), None);
    }

    #[test]
    fn parses_meminfo_in_bytes() {
        let meminfo = "MemTotal:       2048 kB\nMemFree: 1 kB\nMemAvailable:   1024 kB\nSwapTotal: 512 kB\nSwapFree: 256 kB\n";
        assert_eq!(
            parse_meminfo(meminfo),
            MemInfo {
                total: 2048 * 1024,
                available: 1024 * 1024,
                swap_total: 512 * 1024,
                swap_free: 256 * 1024
            }
        );
    }

    #[test]
    fn sums_host_interfaces_only() {
        let net_dev = "Inter-|   Receive |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes\n\
            lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n\
            eth0: 100 1 0 0 0 0 0 0 200 1 0 0 0 0 0 0\n\
            ens5: 10 1 0 0 0 0 0 0 20 1 0 0 0 0 0 0\n\
            docker0: 5 1 0 0 0 0 0 0 5 1 0 0 0 0 0 0\n\
            veth12ab: 7 1 0 0 0 0 0 0 7 1 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(net_dev), (110, 220));
    }

    #[test]
    fn reads_os_pretty_name() {
        assert_eq!(
            parse_pretty_name("ID=ubuntu\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n"),
            Some("Ubuntu 24.04.1 LTS".to_owned())
        );
        assert_eq!(parse_pretty_name("ID=x"), None);
    }

    #[test]
    fn turns_counters_into_rates() {
        let prev = Reading {
            net_rx: 1_000,
            net_tx: 500,
            ..Default::default()
        };
        let cur = Reading {
            cpu_percent: 120.0,
            net_rx: 5_000,
            net_tx: 400,
            ..Default::default()
        };
        let s = sample(Some(&prev), &cur, Duration::from_secs(2), 7);
        assert_eq!(
            (s.at, s.cpu_percent, s.net_rx_rate, s.net_tx_rate),
            (7, 100.0, 2_000, 0)
        );
        assert_eq!(sample(None, &cur, Duration::ZERO, 0).net_rx_rate, 0);
    }

    struct Fixed(u64);

    impl Probe for Fixed {
        fn read(&mut self) -> Reading {
            self.0 += 1;
            Reading {
                mem_used: self.0,
                ..Default::default()
            }
        }
        fn info(&self) -> HostInfo {
            HostInfo::default()
        }
    }

    #[test]
    fn keeps_a_bounded_history() {
        let stats = SystemStats::with_probe(Fixed(0));
        for _ in 0..HISTORY + 5 {
            stats.sample_now();
        }
        let history = lock(&stats.inner.state).history.clone();
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history.front().map(|s| s.mem_used), Some(6));
    }

    #[test]
    fn procfs_probe_reads_a_mounted_host() {
        let dir = std::env::temp_dir().join(format!("rdockpanel-procfs-{}", std::process::id()));
        let proc_root = dir.join("proc");
        fs::create_dir_all(proc_root.join("1/net")).unwrap();
        fs::create_dir_all(proc_root.join("sys/kernel")).unwrap();
        fs::write(
            proc_root.join("stat"),
            "cpu  100 0 0 900 0 0 0 0\ncpu0 1\ncpu1 1\n",
        )
        .unwrap();
        fs::write(
            proc_root.join("meminfo"),
            "MemTotal: 100 kB\nMemAvailable: 40 kB\n",
        )
        .unwrap();
        fs::write(
            proc_root.join("1/net/dev"),
            "h\nh\neth0: 1 0 0 0 0 0 0 0 2 0\n",
        )
        .unwrap();
        fs::write(proc_root.join("sys/kernel/hostname"), "node-1\n").unwrap();
        fs::write(proc_root.join("uptime"), "3600.5 100\n").unwrap();

        let mut probe = ProcfsProbe::new(&proc_root, &dir);
        let first = probe.read();
        assert_eq!(
            (
                first.cpu_percent,
                first.mem_used,
                first.mem_total,
                first.net_rx
            ),
            (0.0, 60 * 1024, 100 * 1024, 1)
        );
        fs::write(
            proc_root.join("stat"),
            "cpu  150 0 0 950 0 0 0 0\ncpu0 1\ncpu1 1\n",
        )
        .unwrap();
        assert!((probe.read().cpu_percent - 50.0).abs() < 1e-9);
        let info = probe.info();
        assert_eq!(
            (info.hostname.as_str(), info.cpus, info.uptime_secs),
            ("node-1", 2, 3600)
        );

        fs::remove_dir_all(dir).unwrap();
    }
}
