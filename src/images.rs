//! Image discovery and guarded removal, entirely on the Docker runtime.
use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use bollard::{
    Docker,
    models::{ContainerSummary, ImageSummary},
    query_parameters::{
        ListContainersOptionsBuilder, ListImagesOptionsBuilder, RemoveImageOptionsBuilder,
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
