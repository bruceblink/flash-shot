//! Machine-readable, bounded performance samples for release gates.

use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const REPORT_FILE: &str = "performance.jsonl";
const MAX_SAMPLES: usize = 500;
const SCHEMA_VERSION: u32 = 2;

pub const fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturePipelineSample {
    pub shortcut_to_frame_ready: Duration,
    pub shortcut_to_overlay_frame: Duration,
    pub platform_capture: Duration,
    pub display_count: usize,
    pub frame_width: u32,
    pub frame_height: u32,
    pub capture_cpu_copy_count: u32,
    pub render_upload_copy_count: u32,
    pub overlay_image_count: usize,
    pub overlay_upload_bytes: usize,
    pub workspace_upload_bytes: usize,
}

#[derive(Clone)]
pub struct PerformanceRecorder {
    report_path: Arc<PathBuf>,
    write_lock: Arc<Mutex<()>>,
}

impl PerformanceRecorder {
    pub fn new(directory: impl AsRef<Path>) -> io::Result<Self> {
        fs::create_dir_all(directory.as_ref())?;
        Ok(Self {
            report_path: Arc::new(directory.as_ref().join(REPORT_FILE)),
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn record_duration(&self, metric: &'static str, duration: Duration) {
        let entry = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "timestamp_ms": unix_timestamp_ms(),
            "build_profile": build_profile(),
            "type": "duration",
            "metric": metric,
            "unit": "ms",
            "value": duration.as_secs_f64() * 1_000.0,
        });
        if let Err(error) = self.append(entry) {
            log::warn!(
                target: "flash_shot::performance",
                "performance_sample_write_failed metric={metric} error={error}"
            );
        }
    }

    pub fn record_capture_pipeline(&self, sample: CapturePipelineSample) {
        let entry = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "timestamp_ms": unix_timestamp_ms(),
            "build_profile": build_profile(),
            "type": "capture_pipeline",
            "latency_ms": {
                "shortcut_to_frame_ready": duration_ms(sample.shortcut_to_frame_ready),
                "shortcut_to_overlay_frame": duration_ms(sample.shortcut_to_overlay_frame),
                "platform_capture": duration_ms(sample.platform_capture),
            },
            "frame": {
                "width": sample.frame_width,
                "height": sample.frame_height,
                "display_count": sample.display_count,
                "cpu_copy_count": sample
                    .capture_cpu_copy_count
                    .saturating_add(sample.render_upload_copy_count),
                "capture_cpu_copy_count": sample.capture_cpu_copy_count,
                "render_upload_copy_count": sample.render_upload_copy_count,
            },
            "preview_images": {
                "overlay_count": sample.overlay_image_count,
                "overlay_upload_bytes": sample.overlay_upload_bytes,
                "workspace_upload_bytes": sample.workspace_upload_bytes,
                "upload_strategy": "one_bgra_upload_per_display",
            },
        });
        if let Err(error) = self.append(entry) {
            log::warn!(
                target: "flash_shot::performance",
                "capture_pipeline_sample_write_failed error={error}"
            );
        }
    }

    fn append(&self, entry: serde_json::Value) -> io::Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| io::Error::other("performance recorder lock poisoned"))?;

        let mut samples = read_samples(&self.report_path)?;
        samples.push_back(entry.to_string());
        while samples.len() > MAX_SAMPLES {
            samples.pop_front();
        }
        write_samples(&self.report_path, samples)
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn read_samples(path: &Path) -> io::Result<VecDeque<String>> {
    let Some(file) = File::open(path).map(Some).or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(error)
        }
    })?
    else {
        return Ok(VecDeque::new());
    };

    BufReader::new(file).lines().collect()
}

fn write_samples(path: &Path, samples: VecDeque<String>) -> io::Result<()> {
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    for sample in samples {
        writeln!(file, "{sample}")?;
    }
    file.sync_all()?;
    fs::rename(temporary, path)
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::{
        CapturePipelineSample, MAX_SAMPLES, PerformanceRecorder, REPORT_FILE, build_profile,
    };
    use std::{fs, time::Duration};

    fn test_directory(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "flash-shot-performance-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn duration_sample_is_machine_readable() {
        let directory = test_directory("json");
        let recorder = PerformanceRecorder::new(&directory).unwrap();

        recorder.record_duration("startup_to_service_ready", Duration::from_millis(42));

        let contents = fs::read_to_string(directory.join(REPORT_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["build_profile"], build_profile());
        assert_eq!(value["type"], "duration");
        assert_eq!(value["metric"], "startup_to_service_ready");
        assert_eq!(value["unit"], "ms");
        assert_eq!(value["value"], 42.0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn report_has_a_fixed_sample_bound() {
        let directory = test_directory("bound");
        let recorder = PerformanceRecorder::new(&directory).unwrap();

        for value in 0..=MAX_SAMPLES {
            recorder
                .append(serde_json::json!({ "value": value }))
                .expect("sample should be written");
        }

        let contents = fs::read_to_string(directory.join(REPORT_FILE)).unwrap();
        assert_eq!(contents.lines().count(), MAX_SAMPLES);
        let first: serde_json::Value =
            serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(first["value"], 1.0);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capture_pipeline_records_latency_copies_and_preview_images() {
        let directory = test_directory("capture-pipeline");
        let recorder = PerformanceRecorder::new(&directory).unwrap();

        recorder.record_capture_pipeline(CapturePipelineSample {
            shortcut_to_frame_ready: Duration::from_millis(18),
            shortcut_to_overlay_frame: Duration::from_millis(27),
            platform_capture: Duration::from_millis(11),
            display_count: 2,
            frame_width: 4480,
            frame_height: 1440,
            capture_cpu_copy_count: 3,
            render_upload_copy_count: 3,
            overlay_image_count: 2,
            overlay_upload_bytes: 123_456,
            workspace_upload_bytes: 98_765,
        });

        let contents = fs::read_to_string(directory.join(REPORT_FILE)).unwrap();
        let value: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(value["type"], "capture_pipeline");
        assert_eq!(value["build_profile"], build_profile());
        assert_eq!(value["latency_ms"]["shortcut_to_frame_ready"], 18.0);
        assert_eq!(value["latency_ms"]["shortcut_to_overlay_frame"], 27.0);
        assert_eq!(value["latency_ms"]["platform_capture"], 11.0);
        assert_eq!(value["frame"]["display_count"], 2);
        assert_eq!(value["frame"]["cpu_copy_count"], 6);
        assert_eq!(value["frame"]["capture_cpu_copy_count"], 3);
        assert_eq!(value["frame"]["render_upload_copy_count"], 3);
        assert_eq!(value["preview_images"]["overlay_count"], 2);
        assert_eq!(
            value["preview_images"]["upload_strategy"],
            "one_bgra_upload_per_display"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
