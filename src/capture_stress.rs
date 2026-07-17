//! Repeatable virtual-desktop capture stress runner and machine-readable report.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};

use crate::platform::capture::capture_virtual_desktop;

const DEFAULT_ITERATIONS: usize = 100;
const WARMUP_ITERATIONS: usize = 3;
const DEFAULT_MAX_HANDLE_GROWTH: i64 = 8;
const DEFAULT_MAX_THREAD_GROWTH: i64 = 2;
const DEFAULT_MAX_WORKING_SET_GROWTH: i64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StressConfig {
    pub iterations: usize,
    pub output: Option<PathBuf>,
    pub max_handle_growth: i64,
    pub max_thread_growth: i64,
    pub max_working_set_growth: i64,
}

impl Default for StressConfig {
    fn default() -> Self {
        Self {
            iterations: DEFAULT_ITERATIONS,
            output: None,
            max_handle_growth: DEFAULT_MAX_HANDLE_GROWTH,
            max_thread_growth: DEFAULT_MAX_THREAD_GROWTH,
            max_working_set_growth: DEFAULT_MAX_WORKING_SET_GROWTH,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ResourceSnapshot {
    handles: u32,
    threads: u32,
    working_set_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct StressReport {
    value: serde_json::Value,
    passed: bool,
}

impl StressReport {
    pub const fn passed(&self) -> bool {
        self.passed
    }

    pub fn to_pretty_json(&self) -> io::Result<String> {
        serde_json::to_string_pretty(&self.value).map_err(io::Error::other)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_pretty_json()?)
    }
}

pub fn run(config: &StressConfig) -> io::Result<StressReport> {
    if config.iterations == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stress iterations must be greater than zero",
        ));
    }

    for _ in 0..WARMUP_ITERATIONS {
        let capture = capture_virtual_desktop()?;
        capture.frame.encode_png()?;
    }
    let before = resource_snapshot()?;
    let mut durations_ms = Vec::with_capacity(config.iterations);
    let mut frame_width = 0;
    let mut frame_height = 0;
    let mut display_count = 0;
    let mut cpu_copy_count = 0;

    for _ in 0..config.iterations {
        let started_at = Instant::now();
        let capture = capture_virtual_desktop()?;
        capture.frame.encode_png()?;
        durations_ms.push(started_at.elapsed().as_secs_f64() * 1_000.0);
        frame_width = capture.frame.width;
        frame_height = capture.frame.height;
        display_count = capture.display_count;
        cpu_copy_count = capture.frame.cpu_copy_count;
    }

    let after = resource_snapshot()?;
    let handle_growth = i64::from(after.handles) - i64::from(before.handles);
    let thread_growth = i64::from(after.threads) - i64::from(before.threads);
    let working_set_growth = after.working_set_bytes as i64 - before.working_set_bytes as i64;
    let passed = handle_growth <= config.max_handle_growth
        && thread_growth <= config.max_thread_growth
        && working_set_growth <= config.max_working_set_growth;
    durations_ms.sort_by(f64::total_cmp);

    let value = serde_json::json!({
        "schema_version": 1,
        "test": "virtual_desktop_capture_stress",
        "passed": passed,
        "iterations": config.iterations,
        "warmup_iterations": WARMUP_ITERATIONS,
        "frame": {
            "width": frame_width,
            "height": frame_height,
            "display_count": display_count,
            "cpu_copy_count": cpu_copy_count,
        },
        "latency_ms": {
            "min": durations_ms[0],
            "p50": percentile(&durations_ms, 50),
            "p95": percentile(&durations_ms, 95),
            "max": durations_ms[durations_ms.len() - 1],
        },
        "resources": {
            "before": resource_json(before),
            "after": resource_json(after),
            "growth": {
                "handles": handle_growth,
                "threads": thread_growth,
                "working_set_bytes": working_set_growth,
            },
            "limits": {
                "handles": config.max_handle_growth,
                "threads": config.max_thread_growth,
                "working_set_bytes": config.max_working_set_growth,
            },
        },
    });
    Ok(StressReport { value, passed })
}

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

fn resource_json(snapshot: ResourceSnapshot) -> serde_json::Value {
    serde_json::json!({
        "handles": snapshot.handles,
        "threads": snapshot.threads,
        "working_set_bytes": snapshot.working_set_bytes,
    })
}

#[cfg(windows)]
fn resource_snapshot() -> io::Result<ResourceSnapshot> {
    use std::mem::size_of;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount},
        },
    };

    // SAFETY: the pseudo handle is always valid for querying the current process.
    let process = unsafe { GetCurrentProcess() };
    let mut handles = 0;
    // SAFETY: handles points to writable storage for the duration of the call.
    if unsafe { GetProcessHandleCount(process, &mut handles) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: memory declares the correct structure size and is a valid out parameter.
    if unsafe { GetProcessMemoryInfo(process, &mut memory, memory.cb) } == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: snapshot flags and process id follow the ToolHelp API contract.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut threads = 0;
    // SAFETY: snapshot and entry remain valid until enumeration completes.
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    let process_id = unsafe { GetCurrentProcessId() };
    while has_entry {
        if entry.th32OwnerProcessID == process_id {
            threads += 1;
        }
        // SAFETY: continues enumeration using the same valid snapshot and entry.
        has_entry = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    // SAFETY: snapshot is an owned kernel handle returned by CreateToolhelp32Snapshot.
    unsafe { CloseHandle(snapshot) };

    Ok(ResourceSnapshot {
        handles,
        threads,
        working_set_bytes: memory.WorkingSetSize,
    })
}

#[cfg(not(windows))]
fn resource_snapshot() -> io::Result<ResourceSnapshot> {
    Ok(ResourceSnapshot::default())
}

#[cfg(test)]
mod tests {
    use super::{StressConfig, percentile};

    #[test]
    fn defaults_match_the_release_stress_gate() {
        let config = StressConfig::default();
        assert_eq!(config.iterations, 100);
        assert_eq!(config.max_handle_growth, 8);
        assert_eq!(config.max_thread_growth, 2);
        assert_eq!(config.max_working_set_growth, 64 * 1024 * 1024);
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let samples: Vec<f64> = (1..=100).map(|value| value as f64).collect();
        assert_eq!(percentile(&samples, 50), 50.0);
        assert_eq!(percentile(&samples, 95), 95.0);
    }
}
