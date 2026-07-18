//! Summarizes locally recorded startup and capture-latency samples for release gates.

use std::{collections::BTreeMap, fs, io, path::Path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceThresholds {
    pub minimum_samples: usize,
    pub since_timestamp_ms: Option<u128>,
    pub startup_p95_ms: Option<u64>,
    pub shortcut_to_frame_ready_p95_ms: Option<u64>,
    pub shortcut_to_overlay_frame_p95_ms: Option<u64>,
}

impl Default for PerformanceThresholds {
    fn default() -> Self {
        Self {
            minimum_samples: 10,
            since_timestamp_ms: None,
            startup_p95_ms: Some(500),
            shortcut_to_frame_ready_p95_ms: Some(100),
            shortcut_to_overlay_frame_p95_ms: Some(100),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricSummary {
    pub samples: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub limit_ms: Option<u64>,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerformanceReport {
    pub metrics: BTreeMap<String, MetricSummary>,
    pub passed: bool,
}

impl PerformanceReport {
    pub fn to_pretty_json(&self) -> io::Result<String> {
        let metrics: BTreeMap<_, _> = self
            .metrics
            .iter()
            .map(|(name, summary)| {
                (
                    name,
                    serde_json::json!({
                        "samples": summary.samples,
                        "min_ms": summary.min_ms,
                        "p50_ms": summary.p50_ms,
                        "p95_ms": summary.p95_ms,
                        "max_ms": summary.max_ms,
                        "limit_ms": summary.limit_ms,
                        "passed": summary.passed,
                    }),
                )
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "test": "recorded_performance_summary",
            "passed": self.passed,
            "metrics": metrics,
        }))
        .map_err(io::Error::other)
    }
}

pub fn summarize_file(
    path: impl AsRef<Path>,
    thresholds: &PerformanceThresholds,
) -> io::Result<PerformanceReport> {
    summarize_samples(&fs::read_to_string(path)?, thresholds)
}

pub fn summarize_samples(
    input: &str,
    thresholds: &PerformanceThresholds,
) -> io::Result<PerformanceReport> {
    let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (line_number, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "invalid performance sample on line {}: {error}",
                    line_number + 1
                ),
            )
        })?;
        if thresholds.since_timestamp_ms.is_some_and(|since| {
            value
                .get("timestamp_ms")
                .and_then(serde_json::Value::as_u64)
                .is_none_or(|timestamp| u128::from(timestamp) < since)
        }) {
            continue;
        }
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("duration") => {
                let Some(metric) = value.get("metric").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let Some(value) = finite_number(value.get("value")) else {
                    continue;
                };
                if metric == "startup_to_first_frame" {
                    samples.entry(metric.to_owned()).or_default().push(value);
                }
            }
            Some("capture_pipeline") => {
                let Some(latency) = value.get("latency_ms") else {
                    continue;
                };
                for metric in ["shortcut_to_frame_ready", "shortcut_to_overlay_frame"] {
                    if let Some(value) = finite_number(latency.get(metric)) {
                        samples.entry(metric.to_owned()).or_default().push(value);
                    }
                }
            }
            _ => {}
        }
    }

    let required_metrics = [
        ("startup_to_first_frame", thresholds.startup_p95_ms),
        (
            "shortcut_to_frame_ready",
            thresholds.shortcut_to_frame_ready_p95_ms,
        ),
        (
            "shortcut_to_overlay_frame",
            thresholds.shortcut_to_overlay_frame_p95_ms,
        ),
    ];
    for (name, limit) in required_metrics {
        if limit.is_some_and(|_| samples.get(name).map_or(0, Vec::len) < thresholds.minimum_samples)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{name} has fewer than {} samples in the requested window",
                    thresholds.minimum_samples
                ),
            ));
        }
    }
    let metrics: BTreeMap<String, MetricSummary> = required_metrics
        .into_iter()
        .filter_map(|(name, limit)| {
            let mut values = samples.remove(name)?;
            values.sort_by(f64::total_cmp);
            let p95_ms = percentile(&values, 95);
            let passed = limit.is_none_or(|limit| p95_ms <= limit as f64);
            Some((
                name.to_owned(),
                MetricSummary {
                    samples: values.len(),
                    min_ms: values[0],
                    p50_ms: percentile(&values, 50),
                    p95_ms,
                    max_ms: values[values.len() - 1],
                    limit_ms: limit,
                    passed,
                },
            ))
        })
        .collect();
    let passed = metrics
        .values()
        .all(|summary: &MetricSummary| summary.passed);
    Ok(PerformanceReport { metrics, passed })
}

fn finite_number(value: Option<&serde_json::Value>) -> Option<f64> {
    value
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    let index = (sorted.len() * percentile).div_ceil(100).saturating_sub(1);
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::{PerformanceThresholds, summarize_samples};

    #[test]
    fn summarizes_duration_and_capture_pipeline_samples() {
        let input = concat!(
            r#"{"type":"duration","metric":"startup_to_first_frame","value":300.0}"#,
            "\n",
            r#"{"type":"duration","metric":"startup_to_first_frame","value":450.0}"#,
            "\n",
            r#"{"type":"capture_pipeline","latency_ms":{"shortcut_to_frame_ready":80.0,"shortcut_to_overlay_frame":90.0}}"#,
            "\n",
            r#"{"type":"capture_pipeline","latency_ms":{"shortcut_to_frame_ready":95.0,"shortcut_to_overlay_frame":120.0}}"#,
        );
        let report = summarize_samples(
            input,
            &PerformanceThresholds {
                minimum_samples: 0,
                ..PerformanceThresholds::default()
            },
        )
        .unwrap();

        assert_eq!(report.metrics["startup_to_first_frame"].samples, 2);
        assert_eq!(report.metrics["startup_to_first_frame"].p95_ms, 450.0);
        assert_eq!(report.metrics["shortcut_to_frame_ready"].p95_ms, 95.0);
        assert_eq!(report.metrics["shortcut_to_overlay_frame"].p95_ms, 120.0);
        assert!(!report.passed);
    }

    #[test]
    fn ignores_unrelated_and_invalid_measurements() {
        let input = concat!(
            r#"{"type":"duration","metric":"other","value":4.0}"#,
            "\n",
            r#"{"type":"capture_pipeline","latency_ms":{"shortcut_to_frame_ready":-1.0}}"#,
            "\n",
            r#"{"type":"capture_pipeline","latency_ms":{"shortcut_to_frame_ready":10.0}}"#,
        );
        let report = summarize_samples(
            input,
            &PerformanceThresholds {
                minimum_samples: 0,
                ..PerformanceThresholds::default()
            },
        )
        .unwrap();

        assert_eq!(report.metrics.len(), 1);
        assert_eq!(report.metrics["shortcut_to_frame_ready"].samples, 1);
        assert!(report.passed);
    }

    #[test]
    fn requires_enough_samples_for_a_gated_metric() {
        let input = r#"{"type":"duration","metric":"startup_to_first_frame","value":42.0}"#;
        let error = summarize_samples(input, &PerformanceThresholds::default()).unwrap_err();
        assert!(error.to_string().contains("fewer than 10 samples"));
    }

    #[test]
    fn uses_nearest_rank_for_the_p95_gate() {
        let mut input = String::new();
        for value in 1..=10 {
            input.push_str(&format!(
                r#"{{"type":"duration","metric":"startup_to_first_frame","value":{value}}}"#
            ));
            input.push('\n');
            input.push_str(&format!(
                r#"{{"type":"capture_pipeline","latency_ms":{{"shortcut_to_frame_ready":{value},"shortcut_to_overlay_frame":{value}}}}}"#
            ));
            input.push('\n');
        }

        let report = summarize_samples(&input, &PerformanceThresholds::default()).unwrap();

        assert_eq!(report.metrics["startup_to_first_frame"].p95_ms, 10.0);
        assert!(report.passed);
    }
}
