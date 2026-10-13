//! Docker resource accounting and bounded background sampling.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bollard::{
    Docker,
    models::{ContainerCpuStats, ContainerStatsResponse, HostConfig},
    query_parameters::StatsOptionsBuilder,
};
use futures_util::{StreamExt, stream};
use tokio::{
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::{docker::Event, model::Container};

pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
pub const STALE_AFTER: Duration = Duration::from_secs(8);
pub const HISTORY_WINDOW: Duration = Duration::from_secs(120);
pub const HISTORY_SAMPLES: usize = 60;

#[derive(Clone, Debug, Default)]
pub struct Limits {
    pub memory: Option<u64>,
    pub cpu_cores: Option<f64>,
    pub cpuset: Option<String>,
}

impl Limits {
    pub fn from_host_config(config: &HostConfig) -> Self {
        Self {
            memory: config
                .memory
                .filter(|value| *value > 0)
                .map(|value| value as u64),
            cpu_cores: config
                .nano_cpus
                .filter(|value| *value > 0)
                .map(|value| value as f64 / 1_000_000_000.0)
                .or_else(|| {
                    let quota = config.cpu_quota.filter(|value| *value > 0)?;
                    let period = config
                        .cpu_period
                        .filter(|value| *value > 0)
                        .unwrap_or(100_000);
                    Some(quota as f64 / period as f64)
                }),
            cpuset: config.cpuset_cpus.clone().filter(|value| !value.is_empty()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub collected: Instant,
    pub cpu_percent: Option<f64>,
    pub memory_used: Option<u64>,
    /// Docker's effective capacity; this is host capacity when no limit is set.
    pub memory_capacity: Option<u64>,
    /// None means Inspect failed, rather than that no limits were configured.
    pub limits: Option<Limits>,
    pub reset: bool,
}

impl Sample {
    pub fn from_stats(
        stats: &ContainerStatsResponse,
        previous: Option<&ContainerCpuStats>,
        limits: Option<Limits>,
    ) -> Self {
        let current = stats.cpu_stats.as_ref();
        let previous = previous.or(stats.precpu_stats.as_ref());
        let cpu_percent = (|| {
            let current = current?;
            let previous = previous?;
            let usage = current.cpu_usage.as_ref()?;
            let before = previous.cpu_usage.as_ref()?.total_usage?;
            // The first sample and counter resets must not masquerade as idle CPU.
            let system_before = previous.system_cpu_usage.filter(|value| *value > 0)?;
            let system_delta = current
                .system_cpu_usage?
                .checked_sub(system_before)
                .filter(|value| *value > 0)?;
            let cpu_delta = usage.total_usage?.checked_sub(before)?;
            let cores = current
                .online_cpus
                .filter(|value| *value > 0)
                .map(|value| value as usize)
                .or_else(|| {
                    usage
                        .percpu_usage
                        .as_ref()
                        .map(Vec::len)
                        .filter(|value| *value > 0)
                })?;
            Some(cpu_delta as f64 / system_delta as f64 * cores as f64 * 100.0)
        })();
        let reset = current
            .and_then(|value| value.cpu_usage.as_ref())
            .and_then(|value| value.total_usage)
            .zip(
                previous
                    .and_then(|value| value.cpu_usage.as_ref())
                    .and_then(|value| value.total_usage),
            )
            .is_some_and(|(current, previous)| current < previous);
        let memory = stats.memory_stats.as_ref();
        let memory_used = memory.and_then(|memory| {
            let usage = memory.usage?;
            let cache = memory
                .stats
                .as_ref()
                .and_then(|stats| {
                    stats
                        .get("total_inactive_file")
                        .or_else(|| stats.get("inactive_file"))
                })
                .copied()
                .unwrap_or(0);
            // Match Docker CLI on cgroup v1/v2; reject inconsistent cache counters.
            Some(if cache < usage { usage - cache } else { usage })
        });
        Self {
            collected: stats
                .read
                .as_deref()
                .and_then(|read| chrono::DateTime::parse_from_rfc3339(read).ok())
                .map(|read| {
                    let age = (chrono::Utc::now().timestamp_millis() - read.timestamp_millis())
                        .max(0) as u64;
                    Instant::now() - Duration::from_millis(age).min(HISTORY_WINDOW + STALE_AFTER)
                })
                .unwrap_or_else(Instant::now),
            cpu_percent,
            memory_used,
            memory_capacity: memory
                .and_then(|memory| memory.limit)
                .filter(|value| *value > 0),
            limits,
            reset,
        }
    }

    pub fn memory_denominator(&self) -> Option<u64> {
        self.limits
            .as_ref()
            .and_then(|limits| limits.memory)
            .or(self.memory_capacity)
    }

    pub fn memory_percent(&self) -> Option<f64> {
        Some(self.memory_used? as f64 / self.memory_denominator()? as f64 * 100.0)
    }
}

pub fn bytes(value: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = value as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

/// A separate worker task: bounded concurrency, no Docker I/O on the GPUI thread
/// or the command loop. Snapshot changes cancel in-flight rounds (stop/removal).
pub(crate) async fn follow(
    docker: Docker,
    session: u64,
    mut snapshots: watch::Receiver<Vec<Container>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut previous = HashMap::<String, ContainerCpuStats>::new();
    let mut limits = HashMap::<String, (Instant, Limits)>::new();
    loop {
        let containers = snapshots
            .borrow_and_update()
            .iter()
            .filter(|container| container.running())
            .map(|container| container.id.clone())
            .collect::<Vec<_>>();
        previous.retain(|id, _| containers.contains(id));
        limits.retain(|id, _| containers.contains(id));
        let requests = containers.iter().map(|id| {
            let id = id.clone();
            let docker = docker.clone();
            let cached = limits
                .get(&id)
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(30))
                .map(|(_, limits)| limits.clone());
            async move {
                let result = time::timeout(Duration::from_secs(3), async {
                    let mut stats = docker.stats(
                        &id,
                        Some(
                            StatsOptionsBuilder::default()
                                .stream(false)
                                .one_shot(true)
                                .build(),
                        ),
                    );
                    let stats = stats
                        .next()
                        .await
                        .ok_or_else(|| "Docker returned no stats".to_string())?
                        .map_err(|error| error.to_string())?;
                    if stats
                        .memory_stats
                        .as_ref()
                        .and_then(|memory| memory.usage)
                        .is_none()
                        && stats
                            .cpu_stats
                            .as_ref()
                            .and_then(|cpu| cpu.cpu_usage.as_ref())
                            .and_then(|cpu| cpu.total_usage)
                            .is_none()
                    {
                        return Err("Docker returned no resource counters".into());
                    }
                    let inspected = if cached.is_none() {
                        docker
                            .inspect_container(&id, None)
                            .await
                            .ok()
                            .and_then(|inspect| inspect.host_config)
                            .map(|config| Limits::from_host_config(&config))
                    } else {
                        None
                    };
                    Ok::<_, String>((stats, cached.or(inspected.clone()), inspected))
                })
                .await
                .unwrap_or_else(|_| Err("Docker stats timed out".into()));
                (id, result)
            }
        });
        let mut requests = stream::iter(requests.collect::<Vec<_>>()).buffer_unordered(16);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                changed = snapshots.changed() => { if changed.is_err() { return; } break; },
                result = requests.next() => {
                    let Some((id, result)) = result else { break; };
                    let result = result.map(|(stats, configured, inspected)| {
                        if let Some(inspected) = inspected { limits.insert(id.clone(), (Instant::now(), inspected)); }
                        let sample = Sample::from_stats(&stats, previous.get(&id), configured);
                        if let Some(cpu) = stats.cpu_stats { previous.insert(id.clone(), cpu); }
                        sample
                    });
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        sent = events.send(Event::Metrics { session, id, result }) => if sent.is_err() { return; },
                    }
                }
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            changed = snapshots.changed() => if changed.is_err() { return; },
            _ = time::sleep(SAMPLE_INTERVAL) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stats(cache: &str) -> ContainerStatsResponse {
        serde_json::from_value(serde_json::json!({
            "cpu_stats": {"cpu_usage": {"total_usage": 900}, "system_cpu_usage": 2000, "online_cpus": 4},
            "precpu_stats": {"cpu_usage": {"total_usage": 300}, "system_cpu_usage": 1000},
            "memory_stats": {"usage": 1024, "limit": 8192, "stats": {cache: 256}}
        })).unwrap()
    }
    #[test]
    fn cpu_can_exceed_one_core_and_memory_matches_cgroup_versions() {
        for cache in ["total_inactive_file", "inactive_file"] {
            let sample = Sample::from_stats(
                &stats(cache),
                None,
                Some(Limits {
                    memory: Some(2048),
                    ..Default::default()
                }),
            );
            assert_eq!(sample.cpu_percent, Some(240.0));
            assert_eq!(sample.memory_used, Some(768));
            assert_eq!(sample.memory_percent(), Some(37.5));
        }
    }
    #[test]
    fn missing_baseline_and_counter_resets_are_not_zero_usage() {
        let mut stats = stats("inactive_file");
        stats.precpu_stats = None;
        assert_eq!(Sample::from_stats(&stats, None, None).cpu_percent, None);
        let mut previous = stats.cpu_stats.clone().unwrap();
        previous.cpu_usage.as_mut().unwrap().total_usage = Some(1000);
        previous.system_cpu_usage = Some(1000);
        let reset = Sample::from_stats(&stats, Some(&previous), None);
        assert_eq!(reset.cpu_percent, None);
        assert!(reset.reset);
        previous.cpu_usage.as_mut().unwrap().total_usage = Some(900);
        assert_eq!(
            Sample::from_stats(&stats, Some(&previous), None).cpu_percent,
            Some(0.0)
        );
    }
    #[test]
    fn configured_limits_are_distinct_from_docker_capacity() {
        let config = HostConfig {
            memory: Some(0),
            cpu_quota: Some(50_000),
            ..Default::default()
        };
        let limits = Limits::from_host_config(&config);
        assert_eq!(limits.memory, None);
        assert_eq!(limits.cpu_cores, Some(0.5));
        let sample = Sample::from_stats(&stats("inactive_file"), None, Some(limits));
        assert_eq!(sample.memory_denominator(), Some(8192));
        assert_eq!(sample.memory_percent(), Some(9.375));
        assert_eq!(bytes(512 * 1024 * 1024), "512.0 MiB");
    }
    #[test]
    fn sample_freshness_uses_daemon_time_and_legacy_cpu_count_falls_back() {
        let mut stats = stats("inactive_file");
        stats.read = Some("2000-01-01T00:00:00Z".into());
        let cpu = stats.cpu_stats.as_mut().unwrap();
        cpu.online_cpus = None;
        cpu.cpu_usage.as_mut().unwrap().percpu_usage = Some(vec![0; 4]);
        let sample = Sample::from_stats(&stats, None, None);
        assert!(sample.collected.elapsed() >= STALE_AFTER);
        assert_eq!(sample.cpu_percent, Some(240.0));
        stats.memory_stats.as_mut().unwrap().usage = None;
        assert_eq!(
            Sample::from_stats(&stats, None, None).memory_percent(),
            None
        );
    }
}
