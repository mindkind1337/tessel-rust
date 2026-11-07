//! Native status-bar telemetry, port discovery, and Win32 keep-awake controller.
//!
//! Architecture & parity guarantees (`StatusBar.vue`, `resourceUsage.js`, `workspacePorts.js`):
//! - Zero subprocesses on Windows: process trees, working-set / private memory, and CPU deltas use
//!   Win32 Toolhelp32 + `K32GetProcessMemoryInfo` + `GetProcessTimes`; listening TCP sockets use
//!   `iphlpapi.dll` `GetExtendedTcpTable` (`AF_INET` + `AF_INET6`, `TCP_TABLE_OWNER_PID_LISTENER`).
//! - Terminal PTY subtrees are claimed *before* Tessel's own process tree so child shells spawned
//!   by `tessel-rust.exe` are attributed to their terminal sessions without double-counting.
//! - Keep-awake uses `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` on the UI thread
//!   (matching Electron's `powerSaveBlocker.start('prevent-app-suspension')`) and resets on `Drop`.
//! - Bounded single background worker thread with `Condvar` wakeups: `2 s` while Resource Manager
//!   is open, `30 s` in the background while focused, `1.5 s` port debounce when probes change, and
//!   instant wakeup on manual refresh or shutdown.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::sidebar_model::{
    APP_HISTORY_KEY, AppResourceEntry, HISTORY_CAPACITY, HISTORY_STALE_MS, KeepAwakeMode,
    ListeningPortEntry, RESOURCE_BACKGROUND_POLL_MS, RESOURCE_OPEN_POLL_MS, ResourceSnapshot,
    TerminalResourceSnapshot,
};

pub const PORT_BACKGROUND_POLL_MS: u64 = 30_000;
pub const PORT_PROBES_SETTLE_MS: u64 = 1_500;
pub const MAX_ANCESTOR_HOPS: usize = 32;

pub const ES_CONTINUOUS_FLAG: u32 = 0x8000_0000;
pub const ES_SYSTEM_REQUIRED_FLAG: u32 = 0x0000_0001;

// ---------------------------------------------------------------------------
// 1. Keep-Awake Controller (`SetThreadExecutionState`)
// ---------------------------------------------------------------------------

#[must_use]
pub const fn should_keep_awake(mode: KeepAwakeMode, any_agent_working: bool) -> bool {
    match mode {
        KeepAwakeMode::On => true,
        KeepAwakeMode::Agents => any_agent_working,
        KeepAwakeMode::Off => false,
    }
}

pub struct KeepAwakeController {
    applied: bool,
    backend: Box<dyn FnMut(bool) -> bool>,
}

impl Default for KeepAwakeController {
    fn default() -> Self {
        Self::new()
    }
}

impl KeepAwakeController {
    #[must_use]
    pub fn new() -> Self {
        Self::with_backend(apply_os_keep_awake)
    }

    #[must_use]
    pub fn with_backend(backend: impl FnMut(bool) -> bool + 'static) -> Self {
        Self {
            applied: false,
            backend: Box::new(backend),
        }
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.applied
    }

    pub fn sync(&mut self, want_active: bool) -> bool {
        if want_active == self.applied {
            return self.applied;
        }
        let ok = (self.backend)(want_active);
        self.applied = if want_active { ok } else { false };
        self.applied
    }
}

impl Drop for KeepAwakeController {
    fn drop(&mut self) {
        if self.applied {
            let _ = (self.backend)(false);
            self.applied = false;
        }
    }
}

fn apply_os_keep_awake(enable: bool) -> bool {
    #[cfg(windows)]
    {
        let flags = if enable {
            ES_CONTINUOUS_FLAG | ES_SYSTEM_REQUIRED_FLAG
        } else {
            ES_CONTINUOUS_FLAG
        };
        // SAFETY: `SetThreadExecutionState` is a thread-local Win32 power hint with no pointers.
        unsafe { win32::SetThreadExecutionState(flags) != 0 }
    }
    #[cfg(not(windows))]
    {
        let _ = enable;
        true
    }
}

// ---------------------------------------------------------------------------
// 2. Pure Resource & Port Attribution Models
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub exe_path: Option<PathBuf>,
    pub working_set_bytes: u64,
    pub private_bytes: u64,
    pub cpu_ticks_100ns: u64,
    pub creation_time_100ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuSampleState {
    pub ticks_100ns: u64,
    pub creation_time_100ns: u64,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TerminalPidProbe {
    pub id: String,
    pub workspace_key: String,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkspacePortProbe {
    pub key: String,
    pub name: String,
    pub path: PathBuf,
    pub pids: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortProtocol {
    Http,
    Https,
}

#[must_use]
pub const fn infer_port_protocol(port: u16) -> PortProtocol {
    match port {
        443 | 8443 => PortProtocol::Https,
        _ => PortProtocol::Http,
    }
}

#[must_use]
pub fn normalize_connect_host(bind_addr: &str) -> String {
    let trimmed = bind_addr.trim();
    match trimmed {
        "" | "*" | "0.0.0.0" | "::" | "[::]" | "::1" | "[::1]" => "localhost".to_string(),
        "127.0.0.1" => "127.0.0.1".to_string(),
        other if other.contains(':') && !other.starts_with('[') => format!("[{other}]"),
        other => other.to_string(),
    }
}

#[must_use]
pub fn address_for_port(connect_host: &str, port: u16) -> String {
    format!("{connect_host}:{port}")
}

#[must_use]
pub fn url_for_port(connect_host: &str, port: u16) -> String {
    let scheme = match infer_port_protocol(port) {
        PortProtocol::Https => "https",
        PortProtocol::Http => "http",
    };
    format!("{scheme}://{connect_host}:{port}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTcpListener {
    pub bind_addr: String,
    pub port: u16,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedPort {
    pub port: u16,
    pub pid: u32,
    pub process_name: String,
    pub bind_host: String,
    pub connect_host: String,
    pub address: String,
    pub url: String,
    pub workspace_key: Option<String>,
    pub workspace_name: Option<String>,
    pub creation_time_100ns: u64,
    pub can_stop: bool,
}

impl DetectedPort {
    #[must_use]
    pub fn to_summary_entry(&self) -> ListeningPortEntry {
        ListeningPortEntry {
            port: self.port,
            workspace_key: self.workspace_key.clone(),
            process_name: self.process_name.clone(),
            pid: self.pid,
        }
    }
}

fn pid_cpu_percent(proc_info: &RawProcessInfo, prev: Option<&CpuSampleState>, now_ms: u64) -> f64 {
    let Some(prev) = prev else {
        return 0.0;
    };
    if prev.creation_time_100ns != 0
        && proc_info.creation_time_100ns != 0
        && prev.creation_time_100ns != proc_info.creation_time_100ns
    {
        return 0.0;
    }
    let dt_ms = now_ms.saturating_sub(prev.at_ms);
    if dt_ms == 0 || proc_info.cpu_ticks_100ns < prev.ticks_100ns {
        return 0.0;
    }
    let delta_ticks = (proc_info.cpu_ticks_100ns - prev.ticks_100ns) as f64;
    // 1 ms = 10_000 * 100ns ticks.
    let cpu_ms = delta_ticks / 10_000.0;
    ((cpu_ms / (dt_ms as f64)) * 100.0).max(0.0)
}

fn valid_child_edge(parent: &RawProcessInfo, child: &RawProcessInfo) -> bool {
    if parent.pid == 0 || parent.pid == child.pid {
        return false;
    }
    if parent.creation_time_100ns != 0
        && child.creation_time_100ns != 0
        && parent.creation_time_100ns > child.creation_time_100ns
    {
        return false;
    }
    true
}

fn collect_unclaimed_subtree(
    root_pid: u32,
    procs: &HashMap<u32, RawProcessInfo>,
    children_map: &HashMap<u32, Vec<u32>>,
    claimed: &mut HashSet<u32>,
) -> Vec<u32> {
    let mut result = Vec::new();
    if root_pid == 0 || !procs.contains_key(&root_pid) || !claimed.insert(root_pid) {
        return result;
    }
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        result.push(pid);
        if let Some(kids) = children_map.get(&pid)
            && let Some(parent_info) = procs.get(&pid)
        {
            for &kid in kids {
                if let Some(kid_info) = procs.get(&kid)
                    && valid_child_edge(parent_info, kid_info)
                    && claimed.insert(kid)
                {
                    stack.push(kid);
                }
            }
        }
    }
    result
}

#[must_use]
pub fn compute_resource_snapshot(
    procs: &HashMap<u32, RawProcessInfo>,
    prev_cpu: &HashMap<u32, CpuSampleState>,
    now_ms: u64,
    app_pid: u32,
    terminal_probes: &[TerminalPidProbe],
) -> (
    ResourceSnapshot,
    HashMap<String, usize>,
    HashMap<u32, CpuSampleState>,
) {
    let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();
    for info in procs.values() {
        if info.ppid != 0 && info.ppid != info.pid {
            children_map.entry(info.ppid).or_default().push(info.pid);
        }
    }
    for kids in children_map.values_mut() {
        kids.sort_unstable();
    }

    let mut next_cpu: HashMap<u32, CpuSampleState> = HashMap::with_capacity(procs.len());
    for info in procs.values() {
        next_cpu.insert(
            info.pid,
            CpuSampleState {
                ticks_100ns: info.cpu_ticks_100ns,
                creation_time_100ns: info.creation_time_100ns,
                at_ms: now_ms,
            },
        );
    }

    let mut claimed: HashSet<u32> = HashSet::new();
    let mut terminals_out: HashMap<String, TerminalResourceSnapshot> = HashMap::new();
    let mut process_counts: HashMap<String, usize> = HashMap::new();

    // 1. Claim terminal PTY subtrees first so child shells of `tessel-rust` belong to their panes.
    for probe in terminal_probes {
        let Some(root_pid) = probe.pid else {
            continue;
        };
        let subtree = collect_unclaimed_subtree(root_pid, procs, &children_map, &mut claimed);
        if subtree.is_empty() {
            continue;
        }
        let mut mem = 0.0_f64;
        let mut priv_mem = 0.0_f64;
        let mut cpu = 0.0_f64;
        for pid in &subtree {
            if let Some(info) = procs.get(pid) {
                mem += info.working_set_bytes as f64;
                priv_mem += info.private_bytes as f64;
                cpu += pid_cpu_percent(info, prev_cpu.get(pid), now_ms);
            }
        }
        process_counts.insert(probe.id.clone(), subtree.len());
        terminals_out.insert(
            probe.id.clone(),
            TerminalResourceSnapshot {
                id: probe.id.clone(),
                memory_bytes: mem,
                private_bytes: priv_mem,
                cpu_percent: cpu,
            },
        );
    }

    // 2. Claim Tessel's own process (`main`) and any remaining helper children (`other`, e.g. WebView2).
    let app_subtree = collect_unclaimed_subtree(app_pid, procs, &children_map, &mut claimed);
    let mut main_mem = 0.0_f64;
    let mut main_priv = 0.0_f64;
    let mut main_cpu = 0.0_f64;
    let mut other_mem = 0.0_f64;
    let mut other_priv = 0.0_f64;
    let mut other_cpu = 0.0_f64;

    for pid in app_subtree {
        if let Some(info) = procs.get(&pid) {
            let ws = info.working_set_bytes as f64;
            let pv = info.private_bytes as f64;
            let c = pid_cpu_percent(info, prev_cpu.get(&pid), now_ms);
            if pid == app_pid {
                main_mem += ws;
                main_priv += pv;
                main_cpu += c;
            } else {
                other_mem += ws;
                other_priv += pv;
                other_cpu += c;
            }
        }
    }

    let mut app_entries = vec![AppResourceEntry {
        kind: "main".to_string(),
        memory_bytes: main_mem,
        cpu_percent: main_cpu,
    }];
    if other_mem > 0.0 || other_cpu > 0.0 {
        app_entries.push(AppResourceEntry {
            kind: "other".to_string(),
            memory_bytes: other_mem,
            cpu_percent: other_cpu,
        });
    }

    let term_mem_sum: f64 = terminals_out.values().map(|t| t.memory_bytes).sum();
    let term_priv_sum: f64 = terminals_out.values().map(|t| t.private_bytes).sum();
    let term_cpu_sum: f64 = terminals_out.values().map(|t| t.cpu_percent).sum();

    let snapshot = ResourceSnapshot {
        timestamp_ms: now_ms,
        total_memory_bytes: main_mem + other_mem + term_mem_sum,
        total_private_bytes: main_priv + other_priv + term_priv_sum,
        total_cpu_percent: main_cpu + other_cpu + term_cpu_sum,
        app: app_entries,
        terminals: terminals_out,
    };

    (snapshot, process_counts, next_cpu)
}

fn norm_path_for_match(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('/', "\\");
    let stripped = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    stripped.trim_end_matches('\\').to_lowercase()
}

fn is_broad_or_root_path(norm_target: &str, norm_home: Option<&str>) -> bool {
    if norm_target.len() < 4 {
        return true;
    }
    if norm_target.len() <= 3 && norm_target.ends_with(':') {
        return true;
    }
    if let Some(home) = norm_home {
        if norm_target == home {
            return true;
        }
        // Ancestor of home (e.g. `c:\users`):
        if home.starts_with(norm_target) && home.as_bytes().get(norm_target.len()) == Some(&b'\\') {
            return true;
        }
    }
    false
}

fn path_within_boundary(candidate: &str, folder: &str) -> bool {
    candidate == folder
        || (candidate.starts_with(folder) && candidate.as_bytes().get(folder.len()) == Some(&b'\\'))
}

#[must_use]
pub fn attribute_listening_ports(
    listeners: &[RawTcpListener],
    procs: &HashMap<u32, RawProcessInfo>,
    workspaces: &[WorkspacePortProbe],
    home_dir: Option<&Path>,
    app_pid: u32,
) -> Vec<DetectedPort> {
    let mut pid_to_ws: HashMap<u32, usize> = HashMap::new();
    for (idx, ws) in workspaces.iter().enumerate() {
        for &pid in &ws.pids {
            if pid != 0 {
                pid_to_ws.entry(pid).or_insert(idx);
            }
        }
    }

    let norm_home = home_dir.map(norm_path_for_match);
    let norm_ws_paths: Vec<(usize, String)> = workspaces
        .iter()
        .enumerate()
        .map(|(idx, ws)| (idx, norm_path_for_match(&ws.path)))
        .filter(|(_, p)| !is_broad_or_root_path(p, norm_home.as_deref()))
        .collect();

    // Deduplicate dual-stack `(pid, port, connect_host)` keeping concrete bind addresses over wildcards.
    let mut dedup: BTreeMap<(u16, u32, String), &RawTcpListener> = BTreeMap::new();
    for item in listeners {
        if item.port == 0 {
            continue;
        }
        let connect_host = normalize_connect_host(&item.bind_addr);
        let key = (item.port, item.pid, connect_host);
        dedup
            .entry(key)
            .and_modify(|existing| {
                let was_wildcard = matches!(
                    existing.bind_addr.as_str(),
                    "*" | "0.0.0.0" | "::" | "[::]" | ""
                );
                let is_wildcard = matches!(
                    item.bind_addr.as_str(),
                    "*" | "0.0.0.0" | "::" | "[::]" | ""
                );
                if was_wildcard && !is_wildcard {
                    *existing = item;
                }
            })
            .or_insert(item);
    }

    let mut out = Vec::with_capacity(dedup.len());
    for ((port, pid, connect_host), listener) in dedup {
        let proc_info = procs.get(&pid);
        let process_name = proc_info
            .map(|p| {
                let stem = p.name.trim();
                stem.strip_suffix(".exe")
                    .or_else(|| stem.strip_suffix(".EXE"))
                    .unwrap_or(stem)
                    .to_string()
            })
            .unwrap_or_default();
        let creation_time_100ns = proc_info.map_or(0, |p| p.creation_time_100ns);

        // 1. Ancestor walk (`pid -> ppid`, up to MAX_ANCESTOR_HOPS, creation-time verified).
        let mut matched_ws: Option<usize> = None;
        if pid != 0 {
            let mut cur = pid;
            let mut visited = HashSet::new();
            for _ in 0..=MAX_ANCESTOR_HOPS {
                if !visited.insert(cur) {
                    break;
                }
                if let Some(&ws_idx) = pid_to_ws.get(&cur) {
                    matched_ws = Some(ws_idx);
                    break;
                }
                let Some(cur_info) = procs.get(&cur) else {
                    break;
                };
                let parent_pid = cur_info.ppid;
                let Some(parent_info) = procs.get(&parent_pid) else {
                    break;
                };
                if !valid_child_edge(parent_info, cur_info) {
                    break;
                }
                cur = parent_pid;
            }
        }

        // 2. Fallback: executable path boundary inside workspace folder (deepest folder wins).
        if matched_ws.is_none()
            && pid != app_pid
            && let Some(exe_path) = proc_info.and_then(|p| p.exe_path.as_deref())
        {
            let norm_exe = norm_path_for_match(exe_path);
            let mut best_len = 0usize;
            for &(ws_idx, ref folder) in &norm_ws_paths {
                if folder.len() > best_len && path_within_boundary(&norm_exe, folder) {
                    best_len = folder.len();
                    matched_ws = Some(ws_idx);
                }
            }
        }

        let (workspace_key, workspace_name) = match matched_ws {
            Some(idx) => (
                Some(workspaces[idx].key.clone()),
                Some(workspaces[idx].name.clone()),
            ),
            None => (None, None),
        };
        let can_stop = workspace_key.is_some() && pid != 0 && pid != app_pid;
        let address = address_for_port(&connect_host, port);
        let url = url_for_port(&connect_host, port);

        out.push(DetectedPort {
            port,
            pid,
            process_name,
            bind_host: listener.bind_addr.clone(),
            connect_host,
            address,
            url,
            workspace_key,
            workspace_name,
            creation_time_100ns,
            can_stop,
        });
    }

    out.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.pid.cmp(&b.pid)));
    out
}

// ---------------------------------------------------------------------------
// 3. Workspace + App Sparkline Ring History (`createResourceHistory` parity)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct SparklineRings {
    rings: HashMap<String, (Vec<f64>, u64)>,
}

impl SparklineRings {
    fn push(&mut self, key: &str, bytes: f64, now_ms: u64) {
        let entry = self
            .rings
            .entry(key.to_string())
            .or_insert_with(|| (Vec::with_capacity(HISTORY_CAPACITY), now_ms));
        entry.0.push(if bytes.is_finite() { bytes } else { 0.0 });
        if entry.0.len() > HISTORY_CAPACITY {
            let extra = entry.0.len() - HISTORY_CAPACITY;
            entry.0.drain(0..extra);
        }
        entry.1 = now_ms;
    }

    pub fn record(
        &mut self,
        snapshot: &ResourceSnapshot,
        probes: &[TerminalPidProbe],
        now_ms: u64,
    ) {
        let app_mem: f64 = snapshot.app.iter().map(|a| a.memory_bytes).sum();
        self.push(APP_HISTORY_KEY, app_mem, now_ms);

        let mut ws_totals: HashMap<&str, f64> = HashMap::new();
        for probe in probes {
            if let Some(usage) = snapshot.terminals.get(&probe.id) {
                let key = if probe.workspace_key.is_empty() {
                    "__other__"
                } else {
                    probe.workspace_key.as_str()
                };
                *ws_totals.entry(key).or_insert(0.0) += usage.memory_bytes;
            }
        }
        for (ws_key, mem) in ws_totals {
            self.push(ws_key, mem, now_ms);
        }
        self.rings.retain(|key, (_, touched)| {
            key == APP_HISTORY_KEY || now_ms.saturating_sub(*touched) <= HISTORY_STALE_MS
        });
    }

    #[must_use]
    pub fn samples(&self, key: &str) -> &[f64] {
        self.rings.get(key).map_or(&[], |(v, _)| v.as_slice())
    }
}

// ---------------------------------------------------------------------------
// 4. Background Telemetry Worker
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TelemetryRequest {
    pub window_focused: bool,
    pub window_minimized: bool,
    pub show_resources: bool,
    pub resources_popover_open: bool,
    pub show_ports: bool,
    pub ports_popover_open: bool,
    pub terminals: Vec<TerminalPidProbe>,
    pub workspaces: Vec<WorkspacePortProbe>,
}

#[derive(Clone, Debug, Default)]
pub struct TelemetryReport {
    pub resources: Option<ResourceSnapshot>,
    pub process_counts: HashMap<String, usize>,
    pub sparklines: SparklineRings,
    pub ports: Vec<DetectedPort>,
    pub ports_refreshing: bool,
    pub ports_error: String,
    pub last_stop_feedback: Option<String>,
}

struct SharedWorkerState {
    request: TelemetryRequest,
    force_resources: bool,
    force_ports: bool,
    stop_port_req: Option<DetectedPort>,
    probes_changed_at: Option<Instant>,
    shutdown: bool,
    report: Arc<TelemetryReport>,
}

pub struct StatusTelemetry {
    shared: Arc<(Mutex<SharedWorkerState>, Condvar)>,
    last_sent: Option<TelemetryRequest>,
}

impl StatusTelemetry {
    #[must_use]
    pub fn spawn(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let initial = SharedWorkerState {
            request: TelemetryRequest::default(),
            force_resources: true,
            force_ports: true,
            stop_port_req: None,
            probes_changed_at: None,
            shutdown: false,
            report: Arc::new(TelemetryReport {
                ports_refreshing: true,
                ..TelemetryReport::default()
            }),
        };
        let shared = Arc::new((Mutex::new(initial), Condvar::new()));
        let worker_shared = shared.clone();
        let _ = std::thread::Builder::new()
            .name("status-telemetry".into())
            .spawn(move || telemetry_worker_loop(worker_shared, Arc::new(wake)));
        Self {
            shared,
            last_sent: None,
        }
    }

    pub fn sync_and_read(&mut self, next: &TelemetryRequest) -> Arc<TelemetryReport> {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let changed = self.last_sent.as_ref() != Some(next);
        if changed {
            let prev = self.last_sent.as_ref();
            let opened_res =
                next.resources_popover_open && !prev.is_some_and(|p| p.resources_popover_open);
            let regained_focus = next.window_focused && !prev.is_some_and(|p| p.window_focused);
            let enabled_res = next.show_resources && !prev.is_some_and(|p| p.show_resources);
            if opened_res || regained_focus || enabled_res {
                guard.force_resources = true;
            }
            let opened_ports =
                next.ports_popover_open && !prev.is_some_and(|p| p.ports_popover_open);
            let enabled_ports = next.show_ports && !prev.is_some_and(|p| p.show_ports);
            if opened_ports || enabled_ports {
                guard.force_ports = true;
            }
            let probes_changed = prev
                .is_some_and(|p| p.workspaces != next.workspaces || p.terminals != next.terminals);
            if probes_changed {
                guard.probes_changed_at = Some(Instant::now());
            }
            guard.request = next.clone();
            self.last_sent = Some(next.clone());
            cvar.notify_one();
        }
        guard.report.clone()
    }

    pub fn refresh_ports_now(&self) {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.force_ports = true;
        let mut rep = (*guard.report).clone();
        rep.ports_refreshing = true;
        guard.report = Arc::new(rep);
        cvar.notify_one();
    }

    #[must_use]
    pub fn latest(&self) -> Arc<TelemetryReport> {
        let (lock, _) = &*self.shared;
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.report.clone()
    }

    pub fn request_stats_refresh(&self) {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.force_resources = true;
        guard.force_ports = true;
        cvar.notify_one();
    }

    pub fn request_stop_port(&self, port: DetectedPort) {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.stop_port_req = Some(port);
        cvar.notify_one();
    }

    pub fn clear_stop_feedback(&self) {
        let (lock, _) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        if guard.report.last_stop_feedback.is_some() {
            let mut rep = (*guard.report).clone();
            rep.last_stop_feedback = None;
            guard.report = Arc::new(rep);
        }
    }
}

impl Drop for StatusTelemetry {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        guard.shutdown = true;
        cvar.notify_all();
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn telemetry_worker_loop(
    shared: Arc<(Mutex<SharedWorkerState>, Condvar)>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let app_pid = std::process::id();
    let home_dir = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
    let mut prev_cpu: HashMap<u32, CpuSampleState> = HashMap::new();
    let mut sparklines = SparklineRings::default();
    let mut last_res_at: Option<Instant> = None;
    let mut last_ports_at: Option<Instant> = None;

    let (lock, cvar) = &*shared;
    loop {
        let (req, do_res, do_ports, stop_req) = {
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if guard.shutdown {
                    return;
                }
                let now = Instant::now();
                let res_in_use = !guard.request.window_minimized
                    && (guard.request.resources_popover_open || guard.request.window_focused);
                let res_interval = if guard.request.resources_popover_open {
                    Duration::from_millis(RESOURCE_OPEN_POLL_MS)
                } else {
                    Duration::from_millis(RESOURCE_BACKGROUND_POLL_MS)
                };
                let want_res = (guard.request.show_resources
                    || guard.request.resources_popover_open)
                    && res_in_use
                    && (guard.force_resources
                        || last_res_at
                            .is_none_or(|t| now.saturating_duration_since(t) >= res_interval));

                let ports_in_use = !guard.request.window_minimized
                    && (guard.request.show_ports || guard.request.ports_popover_open);
                let settle_due = guard.probes_changed_at.is_some_and(|t| {
                    now.saturating_duration_since(t) >= Duration::from_millis(PORT_PROBES_SETTLE_MS)
                });
                let ports_due = last_ports_at.is_none_or(|t| {
                    now.saturating_duration_since(t)
                        >= Duration::from_millis(PORT_BACKGROUND_POLL_MS)
                });
                let want_ports = ports_in_use && (guard.force_ports || settle_due || ports_due);

                if want_res || want_ports || guard.stop_port_req.is_some() {
                    if want_res {
                        guard.force_resources = false;
                    }
                    if want_ports {
                        guard.force_ports = false;
                        guard.probes_changed_at = None;
                    }
                    let stop_req = guard.stop_port_req.take();
                    break (guard.request.clone(), want_res, want_ports, stop_req);
                }

                // Compute next wakeup deadline cleanly without busy-polling.
                let mut wait = Duration::from_secs(30);
                if (guard.request.show_resources || guard.request.resources_popover_open)
                    && res_in_use
                    && let Some(last) = last_res_at
                {
                    wait =
                        wait.min(res_interval.saturating_sub(now.saturating_duration_since(last)));
                }
                if ports_in_use {
                    if let Some(changed) = guard.probes_changed_at {
                        wait = wait.min(
                            Duration::from_millis(PORT_PROBES_SETTLE_MS)
                                .saturating_sub(now.saturating_duration_since(changed)),
                        );
                    }
                    if let Some(last) = last_ports_at {
                        wait = wait.min(
                            Duration::from_millis(PORT_BACKGROUND_POLL_MS)
                                .saturating_sub(now.saturating_duration_since(last)),
                        );
                    }
                }
                let (next_guard, _) = cvar
                    .wait_timeout(guard, wait.max(Duration::from_millis(50)))
                    .unwrap_or_else(|e| e.into_inner());
                guard = next_guard;
            }
        };

        let mut stop_feedback = None;
        let mut need_ports = do_ports;
        if let Some(target) = stop_req {
            stop_feedback = Some(stop_workspace_port_verified(
                &target,
                &req.workspaces,
                home_dir.as_deref(),
                app_pid,
            ));
            need_ports = true;
        }

        let procs = if do_res || need_ports {
            collect_os_processes()
        } else {
            HashMap::new()
        };

        let mut new_res = None;
        let mut new_counts = None;
        if do_res {
            let now_ms = now_epoch_ms();
            let (snap, counts, next_cpu) =
                compute_resource_snapshot(&procs, &prev_cpu, now_ms, app_pid, &req.terminals);
            prev_cpu = next_cpu;
            sparklines.record(&snap, &req.terminals, now_ms);
            new_res = Some(snap);
            new_counts = Some(counts);
            last_res_at = Some(Instant::now());
        }

        let mut new_ports = None;
        if need_ports {
            new_ports = Some(match collect_os_tcp_listeners() {
                Ok(listeners) => Ok(attribute_listening_ports(
                    &listeners,
                    &procs,
                    &req.workspaces,
                    home_dir.as_deref(),
                    app_pid,
                )),
                Err(err) => Err(err),
            });
            last_ports_at = Some(Instant::now());
        }

        {
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            if guard.shutdown {
                return;
            }
            let mut report = (*guard.report).clone();
            if let Some(snap) = new_res {
                report.resources = Some(snap);
                report.sparklines = sparklines.clone();
            }
            if let Some(counts) = new_counts {
                report.process_counts = counts;
            }
            if let Some(port_res) = new_ports {
                report.ports_refreshing = false;
                match port_res {
                    Ok(list) => {
                        report.ports = list;
                        report.ports_error.clear();
                    }
                    Err(e) => {
                        report.ports_error = e;
                    }
                }
            }
            if let Some(msg) = stop_feedback {
                report.last_stop_feedback = Some(msg);
            }
            guard.report = Arc::new(report);
        }
        wake();
    }
}

fn stop_workspace_port_verified(
    target: &DetectedPort,
    workspaces: &[WorkspacePortProbe],
    home_dir: Option<&Path>,
    app_pid: u32,
) -> String {
    if !target.can_stop
        || target.pid == 0
        || target.pid == app_pid
        || target.workspace_key.is_none()
    {
        return "Refused to stop external or application process.".to_string();
    }
    let procs = collect_os_processes();
    let Ok(listeners) = collect_os_tcp_listeners() else {
        return "Could not verify port owner before stopping.".to_string();
    };
    let fresh = attribute_listening_ports(&listeners, &procs, workspaces, home_dir, app_pid);
    let Some(live) = fresh.iter().find(|p| {
        p.port == target.port
            && p.pid == target.pid
            && p.workspace_key.is_some()
            && p.can_stop
            && (target.creation_time_100ns == 0
                || p.creation_time_100ns == target.creation_time_100ns)
    }) else {
        return format!(
            "Port {} is no longer owned by that workspace process.",
            target.port
        );
    };
    terminate_os_process(live.pid, target.creation_time_100ns, target.port)
}

// ---------------------------------------------------------------------------
// 5. Platform Collectors (Win32 Direct APIs / Unix Fallbacks)
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[allow(clippy::upper_case_acronyms)]
mod win32 {
    use super::{RawProcessInfo, RawTcpListener};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    pub type HANDLE = *mut core::ffi::c_void;
    pub type BOOL = i32;

    const INVALID_HANDLE_VALUE: HANDLE = (-1isize) as HANDLE;
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const SYNCHRONIZE: u32 = 0x0010_0000;

    const AF_INET: u32 = 2;
    const AF_INET6: u32 = 23;
    const TCP_TABLE_OWNER_PID_LISTENER: u32 = 3;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FILETIME {
        dw_low_date_time: u32,
        dw_high_date_time: u32,
    }

    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[cfg(target_pointer_width = "64")]
    const _: () = assert!(size_of::<PROCESSENTRY32W>() == 568);

    #[repr(C)]
    struct PROCESS_MEMORY_COUNTERS_EX {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MIB_TCPROW_OWNER_PID {
        dw_state: u32,
        dw_local_addr: u32,
        dw_local_port: u32,
        dw_remote_addr: u32,
        dw_remote_port: u32,
        dw_owning_pid: u32,
    }

    const _: () = assert!(size_of::<MIB_TCPROW_OWNER_PID>() == 24);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MIB_TCP6ROW_OWNER_PID {
        uc_local_addr: [u8; 16],
        dw_local_scope_id: u32,
        dw_local_port: u32,
        uc_remote_addr: [u8; 16],
        dw_remote_scope_id: u32,
        dw_remote_port: u32,
        dw_state: u32,
        dw_owning_pid: u32,
    }

    const _: () = assert!(size_of::<MIB_TCP6ROW_OWNER_PID>() == 56);

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn SetThreadExecutionState(es_flags: u32) -> u32;
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> HANDLE;
        fn Process32FirstW(h_snapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
        fn Process32NextW(h_snapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> BOOL;
        fn OpenProcess(
            dw_desired_access: u32,
            b_inherit_handle: BOOL,
            dw_process_id: u32,
        ) -> HANDLE;
        fn CloseHandle(h_object: HANDLE) -> BOOL;
        fn K32GetProcessMemoryInfo(
            process: HANDLE,
            ppsmem_counters: *mut PROCESS_MEMORY_COUNTERS_EX,
            cb: u32,
        ) -> BOOL;
        fn GetProcessTimes(
            h_process: HANDLE,
            lp_creation_time: *mut FILETIME,
            lp_exit_time: *mut FILETIME,
            lp_kernel_time: *mut FILETIME,
            lp_user_time: *mut FILETIME,
        ) -> BOOL;
        fn QueryFullProcessImageNameW(
            h_process: HANDLE,
            dw_flags: u32,
            lp_exe_name: *mut u16,
            lpdw_size: *mut u32,
        ) -> BOOL;
        fn TerminateProcess(h_process: HANDLE, u_exit_code: u32) -> BOOL;
        fn WaitForSingleObject(h_handle: HANDLE, dw_milliseconds: u32) -> u32;
    }

    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn GetExtendedTcpTable(
            p_tcp_table: *mut u8,
            pdw_size: *mut u32,
            b_order: BOOL,
            ul_af: u32,
            table_class: u32,
            reserved: u32,
        ) -> u32;
    }

    const fn ft_u64(ft: FILETIME) -> u64 {
        ((ft.dw_high_date_time as u64) << 32) | (ft.dw_low_date_time as u64)
    }

    pub fn collect_processes() -> HashMap<u32, RawProcessInfo> {
        let mut map = HashMap::new();
        // SAFETY: `CreateToolhelp32Snapshot` returns either `INVALID_HANDLE_VALUE` or an owned handle.
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap.is_null() || snap == INVALID_HANDLE_VALUE {
            return map;
        }
        // SAFETY: zero-initialized `PROCESSENTRY32W` with `dw_size` populated per Win32 contract.
        let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
        entry.dw_size = size_of::<PROCESSENTRY32W>() as u32;

        // SAFETY: `snap` is valid and `entry` points to writable `PROCESSENTRY32W`.
        let mut ok = unsafe { Process32FirstW(snap, &mut entry) };
        while ok != 0 {
            let pid = entry.th32_process_id;
            let ppid = entry.th32_parent_process_id;
            let nul = entry
                .sz_exe_file
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.sz_exe_file.len());
            let name = OsString::from_wide(&entry.sz_exe_file[..nul])
                .to_string_lossy()
                .into_owned();

            let mut ws_bytes = 0u64;
            let mut priv_bytes = 0u64;
            let mut ticks = 0u64;
            let mut creation = 0u64;
            let mut exe_path = None;

            if pid != 0 {
                // SAFETY: `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION` is read-only.
                let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
                if !h.is_null() {
                    // SAFETY: `pmc` is zeroed with `cb` set to exact struct size.
                    let mut pmc: PROCESS_MEMORY_COUNTERS_EX = unsafe { zeroed() };
                    pmc.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
                    if unsafe {
                        K32GetProcessMemoryInfo(
                            h,
                            &mut pmc,
                            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                        )
                    } != 0
                    {
                        ws_bytes = pmc.working_set_size as u64;
                        priv_bytes = pmc.private_usage as u64;
                    }
                    // SAFETY: all 4 `FILETIME` out-pointers are valid stack structs.
                    let (mut ct, mut et, mut kt, mut ut): (FILETIME, FILETIME, FILETIME, FILETIME) =
                        unsafe { zeroed() };
                    if unsafe { GetProcessTimes(h, &mut ct, &mut et, &mut kt, &mut ut) } != 0 {
                        creation = ft_u64(ct);
                        ticks = ft_u64(kt).saturating_add(ft_u64(ut));
                    }
                    let mut path_buf = [0u16; 1024];
                    let mut path_len = path_buf.len() as u32;
                    if unsafe {
                        QueryFullProcessImageNameW(h, 0, path_buf.as_mut_ptr(), &mut path_len)
                    } != 0
                        && (path_len as usize) <= path_buf.len()
                    {
                        exe_path = Some(PathBuf::from(OsString::from_wide(
                            &path_buf[..path_len as usize],
                        )));
                    }
                    let _ = unsafe { CloseHandle(h) };
                }
            }

            map.insert(
                pid,
                RawProcessInfo {
                    pid,
                    ppid,
                    name,
                    exe_path,
                    working_set_bytes: ws_bytes,
                    private_bytes: priv_bytes,
                    cpu_ticks_100ns: ticks,
                    creation_time_100ns: creation,
                },
            );

            entry.dw_size = size_of::<PROCESSENTRY32W>() as u32;
            ok = unsafe { Process32NextW(snap, &mut entry) };
        }
        let _ = unsafe { CloseHandle(snap) };
        map
    }

    fn fetch_tcp_table_bytes(af: u32) -> Result<Vec<u8>, String> {
        let mut size: u32 = 0;
        // SAFETY: sizing probe with null buffer sets `size`.
        let first = unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                af,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if first != 0 && first != ERROR_INSUFFICIENT_BUFFER {
            return Ok(Vec::new());
        }
        for _ in 0..4 {
            let alloc = (size as usize).max(256) + 512;
            let mut buf = vec![0u8; alloc];
            let mut actual = alloc as u32;
            // SAFETY: `buf` has `actual` writable bytes.
            let rc = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr(),
                    &mut actual,
                    0,
                    af,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if rc == 0 {
                buf.truncate(actual as usize);
                return Ok(buf);
            }
            if rc == ERROR_INSUFFICIENT_BUFFER {
                size = actual;
                continue;
            }
            return Err(format!("GetExtendedTcpTable(af={af}) error {rc}"));
        }
        Err("TCP table grew repeatedly during scan".to_string())
    }

    pub fn collect_tcp_listeners() -> Result<Vec<RawTcpListener>, String> {
        let mut out = Vec::new();

        // IPv4 listeners:
        let v4 = fetch_tcp_table_bytes(AF_INET)?;
        if v4.len() >= 4 {
            let count = u32::from_ne_bytes([v4[0], v4[1], v4[2], v4[3]]) as usize;
            let row_sz = size_of::<MIB_TCPROW_OWNER_PID>();
            let max_rows = (v4.len() - 4) / row_sz;
            for i in 0..count.min(max_rows) {
                let off = 4 + i * row_sz;
                // SAFETY: bounds checked above; `read_unaligned` handles `u8` buffer alignment.
                let row: MIB_TCPROW_OWNER_PID =
                    unsafe { std::ptr::read_unaligned(v4.as_ptr().add(off).cast()) };
                let port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                let ip = std::net::Ipv4Addr::from(row.dw_local_addr.to_ne_bytes());
                out.push(RawTcpListener {
                    bind_addr: ip.to_string(),
                    port,
                    pid: row.dw_owning_pid,
                });
            }
        }

        // IPv6 listeners:
        if let Ok(v6) = fetch_tcp_table_bytes(AF_INET6)
            && v6.len() >= 4
        {
            let count = u32::from_ne_bytes([v6[0], v6[1], v6[2], v6[3]]) as usize;
            let row_sz = size_of::<MIB_TCP6ROW_OWNER_PID>();
            let max_rows = (v6.len() - 4) / row_sz;
            for i in 0..count.min(max_rows) {
                let off = 4 + i * row_sz;
                // SAFETY: bounds checked above; `read_unaligned` handles `u8` buffer alignment.
                let row: MIB_TCP6ROW_OWNER_PID =
                    unsafe { std::ptr::read_unaligned(v6.as_ptr().add(off).cast()) };
                let port = u16::from_be((row.dw_local_port & 0xFFFF) as u16);
                let ip = std::net::Ipv6Addr::from(row.uc_local_addr);
                out.push(RawTcpListener {
                    bind_addr: ip.to_string(),
                    port,
                    pid: row.dw_owning_pid,
                });
            }
        }

        Ok(out)
    }

    pub fn terminate_process_verified(pid: u32, expected_creation_100ns: u64, port: u16) -> String {
        // SAFETY: opens target PID for creation-time check, termination, and exit wait.
        let h = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE,
                0,
                pid,
            )
        };
        if h.is_null() {
            return format!("Could not open process {pid} on port {port}.");
        }
        if expected_creation_100ns != 0 {
            let (mut ct, mut et, mut kt, mut ut): (FILETIME, FILETIME, FILETIME, FILETIME) =
                unsafe { zeroed() };
            if unsafe { GetProcessTimes(h, &mut ct, &mut et, &mut kt, &mut ut) } != 0
                && ft_u64(ct) != expected_creation_100ns
            {
                let _ = unsafe { CloseHandle(h) };
                return format!("Process {pid} changed before stop; aborted.");
            }
        }
        let ok = unsafe { TerminateProcess(h, 1) } != 0;
        if ok {
            let _ = unsafe { WaitForSingleObject(h, 3_000) };
        }
        let _ = unsafe { CloseHandle(h) };
        if ok {
            format!("Stopped process on :{port}")
        } else {
            format!("Failed to stop process {pid} on :{port}")
        }
    }
}

fn collect_os_processes() -> HashMap<u32, RawProcessInfo> {
    #[cfg(windows)]
    {
        win32::collect_processes()
    }
    #[cfg(not(windows))]
    {
        HashMap::new()
    }
}

fn collect_os_tcp_listeners() -> Result<Vec<RawTcpListener>, String> {
    #[cfg(windows)]
    {
        win32::collect_tcp_listeners()
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

fn terminate_os_process(pid: u32, expected_creation_100ns: u64, port: u16) -> String {
    #[cfg(windows)]
    {
        win32::terminate_process_verified(pid, expected_creation_100ns, port)
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, expected_creation_100ns);
        format!("Stopping port :{port} is unsupported on this platform")
    }
}
