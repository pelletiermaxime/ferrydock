//! Image discovery and guarded removal, entirely on the Docker runtime.
use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use bollard::{
    Docker,
    models::{ContainerSummary, ImageHistoryResponseItem, ImageSummary},
    query_parameters::{
        ListContainersOptionsBuilder, ListImagesOptionsBuilder, PruneImagesOptions,
        PruneImagesOptionsBuilder, RemoveImageOptionsBuilder,
    },
};
use tokio::{
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

use crate::docker::Event;

#[derive(Clone, Debug)]
pub struct ImageContainer {
    pub id: String,
    pub name: String,
    pub state: String,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub id: String,
    pub tags: Vec<String>,
    pub digests: Vec<String>,
    pub size: Option<u64>,
    pub created: i64,
    pub containers: Arc<[ImageContainer]>,
}

#[derive(Clone, Debug)]
pub struct RemovalResult {
    pub id: String,
    pub result: Result<(), String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruneMode {
    Untagged,
    AllUnused,
}

impl PruneMode {
    fn options(self) -> PruneImagesOptions {
        PruneImagesOptionsBuilder::default()
            .filters(&HashMap::from([(
                "dangling",
                vec![if self == Self::Untagged {
                    "true"
                } else {
                    "false"
                }],
            )]))
            .build()
    }
}

pub async fn prune(docker: &Docker, mode: PruneMode) -> Result<Option<u64>> {
    // Let Docker decide eligibility, including stopped-container
    // references. Inventory selection and search never limit a daemon prune.
    let response = time::timeout(
        Duration::from_secs(120),
        docker.prune_images(Some(mode.options())),
    )
    .await
    .context("Image pruning timed out; refresh to check Docker state")??;
    Ok(response
        .space_reclaimed
        .and_then(|bytes| u64::try_from(bytes).ok()))
}

impl Image {
    pub fn short_id(&self) -> &str {
        let id = self.id.strip_prefix("sha256:").unwrap_or(&self.id);
        id.get(..12).unwrap_or(id)
    }

    pub fn name(&self) -> &str {
        self.tags
            .first()
            .map(String::as_str)
            .unwrap_or("Untagged image")
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.id.to_lowercase().contains(&query)
            || self
                .tags
                .iter()
                .chain(&self.digests)
                .any(|tag| tag.to_lowercase().contains(&query))
    }

    pub fn size_label(&self) -> String {
        self.size
            .map(|bytes| format!("{:.1} MiB", bytes as f64 / 1_048_576.0))
            .unwrap_or_else(|| "Unavailable".into())
    }

    pub fn created_label(&self) -> String {
        self.created_timestamp()
            .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
            .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "Unavailable".into())
    }

    pub fn age_label(&self) -> String {
        let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
            return "Unavailable".into();
        };
        self.age_label_at(now.as_secs() as i64)
    }

    fn age_label_at(&self, now: i64) -> String {
        crate::resource_age::label(self.created_timestamp(), now)
    }

    pub fn created_timestamp(&self) -> Option<i64> {
        (self.created > 0 && chrono::DateTime::from_timestamp(self.created, 0).is_some())
            .then_some(self.created)
    }
}

fn from_summaries(
    images: Vec<ImageSummary>,
    containers: Vec<ContainerSummary>,
) -> Result<Vec<Image>> {
    // Refuse oversized inventories rather than retaining or silently truncating
    // references that determine whether removal is available.
    if images.len() > 20_000 || containers.len() > 50_000 {
        bail!(
            "This Docker inventory exceeds FerryDock's image view limit (20,000 images / 50,000 containers)"
        );
    }
    let mut references = HashMap::<String, Vec<ImageContainer>>::new();
    for container in containers {
        let id = container.id.unwrap_or_default();
        let name = container
            .names
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_else(|| id.chars().take(12).collect())
            .trim_start_matches('/')
            .to_owned();
        references
            .entry(container.image_id.unwrap_or_default())
            .or_default()
            .push(ImageContainer {
                id,
                name,
                state: container
                    .state
                    .map(|state| state.to_string())
                    .unwrap_or_else(|| "unknown".into()),
            });
    }
    let mut images = images
        .into_iter()
        .map(|summary| {
            let mut tags = summary
                .repo_tags
                .into_iter()
                .filter(|tag| tag != "<none>:<none>")
                .collect::<Vec<_>>();
            tags.sort();
            tags.dedup();
            let mut containers = references.remove(&summary.id).unwrap_or_default();
            containers.sort_by(|a, b| a.name.cmp(&b.name));
            Image {
                id: summary.id,
                tags,
                digests: summary.repo_digests,
                size: u64::try_from(summary.size).ok(),
                created: summary.created,
                containers: containers.into(),
            }
        })
        .collect::<Vec<_>>();
    images.sort_by(|a, b| a.name().cmp(b.name()).then(a.id.cmp(&b.id)));
    Ok(images)
}

pub async fn snapshot(docker: &Docker) -> Result<Vec<Image>> {
    time::timeout(Duration::from_secs(15), async {
        let (images, containers) = tokio::try_join!(
            docker.list_images(Some(
                ListImagesOptionsBuilder::default()
                    .all(true)
                    .digests(true)
                    .build()
            )),
            docker.list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build()
            )),
        )?;
        from_summaries(images, containers)
    })
    .await
    .context("Image discovery timed out")?
}

pub async fn history(docker: &Docker, id: &str) -> Result<Vec<ImageHistoryResponseItem>> {
    let rows = time::timeout(Duration::from_secs(15), docker.image_history(id))
        .await
        .context("Image history timed out")??;
    let bytes: usize = rows
        .iter()
        .map(|row| {
            row.id.len()
                + row.created_by.len()
                + row.comment.len()
                + row.tags.iter().map(String::len).sum::<usize>()
        })
        .sum();
    if rows.len() > 2_000 || bytes > 4 * 1_048_576 {
        bail!("Image history exceeds FerryDock's display limit (2,000 entries / 4 MiB)");
    }
    Ok(rows)
}

pub async fn inspect(docker: &Docker, id: &str) -> Result<String> {
    let value = time::timeout(Duration::from_secs(15), docker.inspect_image(id))
        .await
        .context("Image inspection timed out")??;
    let text = serde_json::to_string_pretty(&value)?;
    if text.len() > 4 * 1_048_576 {
        bail!("Image inspection exceeds FerryDock's display limit (4 MiB)");
    }
    Ok(text)
}

/// One selected image at a time. A newer selection drops both in-flight reads;
/// either result can still be displayed if the other endpoint fails.
pub(crate) async fn follow_details(
    docker: Docker,
    mut requests: watch::Receiver<Option<(String, u64)>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            changed = requests.changed() => if changed.is_err() { return; },
        }
        loop {
            let request = requests.borrow_and_update().clone();
            let Some((id, generation)) = request else {
                break;
            };
            let load = async {
                let history = async {
                    let result = history(&docker, &id)
                        .await
                        .map_err(|error| format!("{error:#}"));
                    let _ = events
                        .send(Event::ImageHistory {
                            id: id.clone(),
                            generation,
                            result,
                        })
                        .await;
                };
                let inspection = async {
                    let result = inspect(&docker, &id)
                        .await
                        .map_err(|error| format!("{error:#}"));
                    let _ = events
                        .send(Event::ImageInspection {
                            id: id.clone(),
                            generation,
                            result,
                        })
                        .await;
                };
                tokio::join!(history, inspection);
            };
            tokio::select! {
                _ = cancel.cancelled() => return,
                changed = requests.changed() => {
                    if changed.is_err() { return; }
                    continue;
                },
                _ = load => break,
            }
        }
    }
}

pub async fn remove(docker: &Docker, id: &str) -> Result<()> {
    remove_many(docker, vec![id.to_owned()])
        .await
        .into_iter()
        .next()
        .context("Missing image removal result")?
        .result
        .map_err(anyhow::Error::msg)
}

pub async fn remove_many(docker: &Docker, ids: Vec<String>) -> Vec<RemovalResult> {
    // One fresh inventory for the batch. Non-forced Docker deletion also
    // rejects references created after this preflight, for every image.
    let inventory = snapshot(docker).await.map(|images| {
        images
            .into_iter()
            .map(|image| (image.id.clone(), image))
            .collect::<HashMap<_, _>>()
    });
    let mut results = Vec::with_capacity(ids.len());
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let result = async {
            let inventory = inventory.as_ref().map_err(|error| anyhow::anyhow!("Image discovery failed: {error:#}"))?;
            let image = inventory.get(&id).context("Image no longer exists; refresh the list")?;
            if !image.containers.is_empty() {
                bail!("Image is used by {} container(s), including stopped containers; remove those containers first", image.containers.len());
            }
            time::timeout(Duration::from_secs(30), docker.remove_image(&id, Some(RemoveImageOptionsBuilder::default().force(false).noprune(true).build()), None))
                .await.context("Image removal timed out; refresh to check Docker state")??;
            Ok::<_, anyhow::Error>(())
        }.await.map_err(|error| format!("{error:#}"));
        results.push(RemovalResult { id, result });
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
                _ = events.send(Event::Images { generation, result }) => {},
            }
            interval.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_labels_handle_unit_boundaries_missing_dates_and_future_clock_skew() {
        let mut image = Image {
            id: "age-fixture".into(),
            tags: vec![],
            digests: vec![],
            size: None,
            created: 1_000_000_000,
            containers: vec![].into(),
        };
        for (elapsed, expected) in [
            (59, "Just now"),
            (60, "1 minute ago"),
            (120, "2 minutes ago"),
            (3_600, "1 hour ago"),
            (2 * 86_400, "2 days ago"),
            (30 * 86_400, "1 month ago"),
            (365 * 86_400, "1 year ago"),
        ] {
            assert_eq!(image.age_label_at(image.created + elapsed), expected);
        }
        assert_eq!(image.age_label_at(image.created - 7_200), "In 2 hours");
        for created in [0, -1, i64::MAX] {
            image.created = created;
            assert_eq!(image.age_label_at(1_000_000_000), "Unavailable");
        }
    }

    #[test]
    fn image_references_use_ids_even_when_tags_move_and_include_stopped_containers() {
        let images = vec![
            ImageSummary {
                id: "sha256:old".into(),
                repo_tags: vec!["<none>:<none>".into()],
                size: -1,
                ..Default::default()
            },
            ImageSummary {
                id: "sha256:new".into(),
                repo_tags: vec!["app:latest".into()],
                size: 1_048_576,
                ..Default::default()
            },
        ];
        let containers = vec![ContainerSummary {
            id: Some("stopped".into()),
            names: Some(vec!["/app-old".into()]),
            image: Some("app:latest".into()),
            image_id: Some("sha256:old".into()),
            state: Some(bollard::models::ContainerSummaryStateEnum::EXITED),
            ..Default::default()
        }];
        let inventory = from_summaries(images, containers).unwrap();
        let old = inventory
            .iter()
            .find(|image| image.id == "sha256:old")
            .unwrap();
        assert!(old.tags.is_empty());
        assert_eq!(old.size_label(), "Unavailable");
        assert_eq!(old.containers[0].name, "app-old");
        assert_eq!(old.containers[0].state, "exited");
        assert!(
            inventory
                .iter()
                .find(|image| image.id == "sha256:new")
                .unwrap()
                .containers
                .is_empty()
        );
        assert!(old.matches("OLD"));
        assert_eq!(
            inventory
                .iter()
                .find(|image| image.matches("APP:latest"))
                .unwrap()
                .size_label(),
            "1.0 MiB"
        );
    }
}
