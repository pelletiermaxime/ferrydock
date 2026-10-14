//! Bounded volume discovery, explicit size gathering, removal and prune on the Docker runtime.
use crate::docker::Event;
use anyhow::{Context, Result, bail};
use bollard::{
    Docker,
    models::{ContainerSummary, Volume as DockerVolume},
    query_parameters::{
        ListContainersOptionsBuilder, ListVolumesOptions, PruneVolumesOptionsBuilder,
        RemoveVolumeOptionsBuilder,
    },
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct VolumeContainer {
    pub id: String,
    pub name: String,
    pub state: String,
}

#[derive(Clone, Debug)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub created: Option<String>,
    pub scope: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub options: BTreeMap<String, String>,
    pub size: Option<u64>,
    pub containers: Arc<[VolumeContainer]>,
}

#[derive(Clone, Debug)]
pub struct RemovalResult {
    pub name: String,
    pub result: Result<(), String>,
}

#[derive(Clone, Debug)]
pub struct PruneResult {
    pub deleted: Vec<String>,
    pub reclaimed: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruneMode {
    Anonymous,
    AllUnused,
}

pub async fn prune(docker: &Docker, mode: PruneMode) -> Result<PruneResult> {
    let version = docker.client_version();
    if (version.major_version, version.minor_version) < (1, 42) {
        bail!("Volume prune modes require Docker API 1.42 or newer (Docker 23+)");
    }
    // Docker checks all references, including stopped containers. Explicit mode
    // filters avoid pruning named volumes in anonymous mode. Search/selection
    // never limit a daemon prune. Older APIs lack this distinction and are refused.
    let docker = docker.clone().with_timeout(Duration::from_secs(120));
    let response = time::timeout(
        Duration::from_secs(120),
        docker.prune_volumes(Some(
            PruneVolumesOptionsBuilder::default()
                .filters(&HashMap::from([(
                    "all",
                    vec![if mode == PruneMode::AllUnused {
                        "true"
                    } else {
                        "false"
                    }],
                )]))
                .build(),
        )),
    )
    .await
    .context("Volume pruning timed out; refresh to check Docker state")??;
    Ok(PruneResult {
        deleted: response.volumes_deleted.unwrap_or_default(),
        reclaimed: response
            .space_reclaimed
            .and_then(|bytes| bytes.try_into().ok()),
    })
}

/// A measurement includes identity so a recreated volume never inherits an old size.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct VolumeSize {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    #[serde(rename = "CreatedAt")]
    pub created: Option<String>,
    #[serde(default, rename = "UsageData", deserialize_with = "read_size")]
    pub size: Option<u64>,
}

fn read_size<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<u64>, D::Error> {
    #[derive(serde::Deserialize)]
    struct Usage {
        #[serde(rename = "Size")]
        size: Option<i64>,
    }
    let usage = <Option<Usage> as serde::Deserialize>::deserialize(input)?;
    Ok(usage
        .and_then(|usage| usage.size)
        .and_then(|size| size.try_into().ok()))
}

impl VolumeSize {
    pub fn matches(&self, volume: &Volume) -> bool {
        self.name == volume.name
            && self.driver == volume.driver
            && self.mountpoint == volume.mountpoint
            && self.created == volume.created
    }
}

#[cfg(unix)]
pub async fn gather_sizes(
    endpoint: &str,
    version: &bollard::ClientVersion,
) -> Result<Vec<VolumeSize>> {
    use http_body_util::{BodyExt, Empty, Limited};
    use hyper::body::Bytes;
    use hyper_util::client::legacy::Client;
    use hyperlocal::{UnixClientExt, UnixConnector, Uri};
    if (version.major_version, version.minor_version) < (1, 42) {
        bail!("Gathering volume sizes requires Docker API 1.42 or newer (Docker 23+)");
    }
    time::timeout(Duration::from_secs(120), async {
        // The volume filter avoids calculating image/container/build-cache usage.
        let path = format!(
            "/v{}.{}/system/df?type=volume&verbose=true",
            version.major_version, version.minor_version
        );
        let client: Client<UnixConnector, Empty<Bytes>> = Client::unix();
        let response = client
            .get(Uri::new(endpoint.trim_start_matches("unix://"), &path).into())
            .await
            .context("Could not request volume sizes from Docker")?;
        let status = response.status();
        let bytes = Limited::new(response.into_body(), 32 * 1024 * 1024)
            .collect()
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Could not read Docker volume sizes (32 MiB response limit): {error}"
                )
            })?
            .to_bytes();
        if !status.is_success() {
            let message = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| status.to_string());
            bail!("Docker could not gather volume sizes: {message}");
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Usage {
            items: Option<Vec<VolumeSize>>,
            total_count: Option<u64>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Response {
            volumes: Option<Vec<VolumeSize>>,
            volume_usage: Option<Usage>,
        }
        let response: Response =
            serde_json::from_slice(&bytes).context("Invalid Docker volume size response")?;
        let sizes = match (response.volume_usage, response.volumes) {
            (Some(usage), _) => match usage.items {
                Some(items) => items,
                None if usage.total_count == Some(0) => Vec::new(),
                _ => bail!("Docker did not return individual volume sizes"),
            },
            (_, Some(items)) => items,
            // Older engines represent an empty inventory as Volumes: null.
            _ if serde_json::from_slice::<serde_json::Value>(&bytes)?
                .get("Volumes")
                .is_some() =>
            {
                Vec::new()
            }
            _ => bail!("Docker did not return individual volume sizes"),
        };
        if sizes.len() > 20_000 {
            bail!("Volume sizes exceed FerryDock's limit (20,000 volumes)");
        }
        Ok(sizes)
    })
    .await
    .context("Gathering volume sizes timed out; previous measurements are kept")?
}

#[cfg(not(unix))]
pub async fn gather_sizes(_: &str, _: &bollard::ClientVersion) -> Result<Vec<VolumeSize>> {
    bail!("Gathering volume sizes requires a local Unix Docker socket")
}

impl Volume {
    pub fn project(&self) -> Option<&str> {
        self.labels
            .get("com.docker.compose.project")
            .map(String::as_str)
    }

    pub fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.driver.to_lowercase().contains(query)
            || self
                .project()
                .is_some_and(|project| project.to_lowercase().contains(query))
    }

    pub fn size_label(&self) -> String {
        self.size
            .map(|bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0))
            .unwrap_or_else(|| "Unavailable".into())
    }

    pub fn created_timestamp(&self) -> Option<i64> {
        self.created
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp())
    }

    pub fn age_label(&self, now: i64) -> String {
        crate::resource_age::label(self.created_timestamp(), now)
    }

    pub fn created_label(&self) -> String {
        self.created_timestamp()
            .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
            .map(|value| value.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "Unavailable".into())
    }
}

fn from_summaries(
    volumes: Vec<DockerVolume>,
    containers: Vec<ContainerSummary>,
) -> Result<Vec<Volume>> {
    if volumes.len() > 20_000 || containers.len() > 50_000 {
        bail!(
            "This Docker inventory exceeds FerryDock's volume view limit (20,000 volumes / 50,000 containers)"
        );
    }
    let mut references = HashMap::<String, Vec<VolumeContainer>>::new();
    let mut reference_count = 0;
    for container in containers {
        let id = container
            .id
            .context("Docker returned a container without an ID; volume references unavailable")?;
        let name = container
            .names
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_else(|| id.chars().take(12).collect())
            .trim_start_matches('/')
            .to_owned();
        let state = container
            .state
            .map(|state| state.to_string())
            .unwrap_or_else(|| "unknown".into());
        let mut seen = HashSet::new();
        for mount in container.mounts.unwrap_or_default() {
            if mount.typ.as_deref() != Some("volume") {
                continue;
            }
            let volume = mount.name.filter(|name| !name.is_empty()).context(
                "Docker returned a volume mount without a name; volume references unavailable",
            )?;
            if !seen.insert(volume.clone()) {
                continue;
            }
            reference_count += 1;
            if reference_count > 100_000 {
                bail!(
                    "This Docker inventory exceeds FerryDock's volume reference limit (100,000 references)"
                );
            }
            references.entry(volume).or_default().push(VolumeContainer {
                id: id.clone(),
                name: name.clone(),
                state: state.clone(),
            });
        }
    }
    let mut inventory = volumes
        .into_iter()
        .map(|volume| {
            let mut containers = references.remove(&volume.name).unwrap_or_default();
            containers.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            Volume {
                name: volume.name,
                driver: volume.driver,
                mountpoint: volume.mountpoint,
                created: volume.created_at,
                scope: volume.scope.map(|scope| scope.to_string()),
                labels: volume.labels.into_iter().collect(),
                options: volume.options.into_iter().collect(),
                size: volume
                    .usage_data
                    .and_then(|usage| u64::try_from(usage.size).ok()),
                containers: containers.into(),
            }
        })
        .collect::<Vec<_>>();
    inventory.sort_by_cached_key(|volume| (volume.name.to_lowercase(), volume.name.clone()));
    Ok(inventory)
}

pub async fn snapshot(docker: &Docker) -> Result<Vec<Volume>> {
    time::timeout(Duration::from_secs(15), async {
        let (volumes, containers) = tokio::try_join!(
            docker.list_volumes(None::<ListVolumesOptions>),
            docker.list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build()
            )),
        )?;
        if let Some(warnings) = volumes.warnings.filter(|warnings| !warnings.is_empty()) {
            bail!(
                "Docker volume inventory is incomplete: {}",
                warnings.join("; ")
            );
        }
        from_summaries(volumes.volumes.unwrap_or_default(), containers)
    })
    .await
    .context("Volume discovery timed out")?
}

pub async fn remove(docker: &Docker, name: &str) -> Result<()> {
    remove_many(docker, vec![name.to_owned()])
        .await
        .into_iter()
        .next()
        .context("Missing volume removal result")?
        .result
        .map_err(anyhow::Error::msg)
}

pub async fn remove_many(docker: &Docker, names: Vec<String>) -> Vec<RemovalResult> {
    let inventory = snapshot(docker).await.map(|volumes| {
        volumes
            .into_iter()
            .map(|volume| (volume.name.clone(), volume))
            .collect::<HashMap<_, _>>()
    });
    let mut results = Vec::with_capacity(names.len());
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        let result = async {
            let inventory = inventory.as_ref().map_err(|error| anyhow::anyhow!("Volume discovery failed: {error:#}"))?;
            let volume = inventory.get(&name).context("Volume no longer exists; refresh the list")?;
            if !volume.containers.is_empty() {
                bail!("Volume is used by {} container(s), including stopped containers; remove those containers first", volume.containers.len());
            }
            // Docker protects references created after preflight too. Never force or prune.
            time::timeout(Duration::from_secs(30), docker.remove_volume(&name, Some(RemoveVolumeOptionsBuilder::default().force(false).build())))
                .await.context("Volume removal timed out; refresh to check Docker state")??;
            Ok::<_, anyhow::Error>(())
        }.await.map_err(|error| format!("{error:#}"));
        results.push(RemovalResult { name, result });
    }
    results
}

pub(crate) async fn follow(
    docker: Docker,
    mut requests: watch::Receiver<Option<u64>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut interval = time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            changed = requests.changed() => if changed.is_err() { return; },
            _ = interval.tick() => {},
        }
        let generation = *requests.borrow_and_update();
        if let Some(generation) = generation {
            let result = tokio::select! {
                _ = cancel.cancelled() => return,
                result = snapshot(&docker) => result.map_err(|error| format!("{error:#}")),
            };
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = events.send(Event::Volumes { generation, result }) => {},
            }
            interval.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerSummaryStateEnum, MountPoint};

    #[test]
    fn references_include_stopped_containers_once_and_exclude_bind_mounts() {
        let volume = DockerVolume {
            name: "demo-data".into(),
            driver: "local".into(),
            labels: [("com.docker.compose.project".into(), "Demo".into())].into(),
            ..Default::default()
        };
        let mount = MountPoint {
            typ: Some("volume".into()),
            name: Some("demo-data".into()),
            ..Default::default()
        };
        let container = ContainerSummary {
            id: Some("stopped".into()),
            names: Some(vec!["/database".into()]),
            state: Some(ContainerSummaryStateEnum::EXITED),
            mounts: Some(vec![
                mount.clone(),
                mount,
                MountPoint {
                    typ: Some("bind".into()),
                    name: Some("demo-data".into()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        let volumes = from_summaries(vec![volume], vec![container]).unwrap();
        assert_eq!(volumes[0].containers.len(), 1);
        assert_eq!(volumes[0].containers[0].state, "exited");
        assert_eq!(volumes[0].containers[0].name, "database");
        assert!(volumes[0].matches("demo"));
        assert!(volumes[0].matches("local"));
        assert_eq!(volumes[0].size_label(), "Unavailable");
        assert_eq!(volumes[0].created_label(), "Unavailable");
    }

    #[test]
    fn incomplete_references_fail_instead_of_marking_volume_unused() {
        let container = ContainerSummary {
            id: Some("container".into()),
            mounts: Some(vec![MountPoint {
                typ: Some("volume".into()),
                ..Default::default()
            }]),
            ..Default::default()
        };
        assert!(from_summaries(vec![], vec![container]).is_err());
        assert!(from_summaries(vec![DockerVolume::default(); 20_001], vec![]).is_err());
    }
    #[test]
    fn age_uses_creation_instant_and_handles_missing_invalid_and_future_dates() {
        let mut volume = from_summaries(
            vec![DockerVolume {
                created_at: Some("2026-10-13T10:00:00-04:00".into()),
                ..Default::default()
            }],
            vec![],
        )
        .unwrap()
        .remove(0);
        let created = volume.created_timestamp().unwrap();
        assert_eq!(volume.created_label(), "2026-10-13 14:00 UTC");
        for (elapsed, label) in [
            (0, "Just now"),
            (60, "1 minute ago"),
            (120, "2 minutes ago"),
            (3600, "1 hour ago"),
            (7200, "2 hours ago"),
            (86400, "1 day ago"),
            (864000, "10 days ago"),
        ] {
            assert_eq!(volume.age_label(created + elapsed), label);
        }
        assert_eq!(volume.age_label(created - 7200), "In 2 hours");
        volume.created = Some("invalid".into());
        assert_eq!(volume.age_label(created), "Unavailable");
        volume.created = None;
        assert_eq!(volume.age_label(created), "Unavailable");
    }
}
