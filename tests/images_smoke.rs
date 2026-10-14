//! Real worker image discovery/removal with owned fixtures supplied by the script.
use anyhow::{Context, Result, bail};
use ferrydock::{
    docker::{self, Backend, Command, Event},
    images::Image,
};
use std::{env, time::Duration};
use tokio::{sync::mpsc, time};

async fn inventory(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    generation: u64,
) -> Result<Vec<Image>> {
    backend.send(Command::Images { generation });
    time::timeout(Duration::from_secs(25), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::Images {
                    generation: current,
                    result,
                } if current == generation => return result.map_err(anyhow::Error::msg),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Timed out waiting for images")?
}

async fn remove(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    id: &str,
    operation: u64,
) -> Result<Result<(), String>> {
    backend.send(Command::RemoveImage {
        id: id.into(),
        operation,
    });
    time::timeout(Duration::from_secs(40), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::ImageRemoved {
                    id: current,
                    operation: current_operation,
                    result,
                } if current == id && current_operation == operation => return Ok(result),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Timed out waiting for image removal")?
}

async fn remove_many(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    ids: Vec<String>,
    operation: u64,
) -> Result<Vec<ferrydock::images::RemovalResult>> {
    backend.send(Command::RemoveImages { ids, operation });
    time::timeout(Duration::from_secs(75), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::ImagesRemoved {
                    operation: current,
                    results,
                } if current == operation => return Ok(results),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Timed out waiting for bulk image removal")?
}

#[tokio::test]
#[ignore = "Creates and removes only the fixtures supplied by scripts/verify-images.sh"]
async fn image_worker_discovery_and_guarded_removal() -> Result<()> {
    let project = env::var("FERRYDOCK_IMAGE_TEST_PROJECT")?;
    let image_id = env::var("FERRYDOCK_IMAGE_TEST_ID")?;
    let tagged_id = env::var("FERRYDOCK_IMAGE_TEST_TAGGED_ID")?;
    let spare_ids = env::var("FERRYDOCK_IMAGE_TEST_SPARE_IDS")?
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(spare_ids.len(), 2);
    let container_id = env::var("FERRYDOCK_IMAGE_TEST_CONTAINER")?;
    let endpoint = env::var("FERRYDOCK_IMAGE_TEST_ENDPOINT")?;
    let connection = docker::resolve_connection().await?;
    assert_eq!(connection.endpoint, endpoint);
    let docker = docker::connect(&connection).await?;
    for id in [&image_id, &tagged_id, &spare_ids[0], &spare_ids[1]] {
        let image = docker.inspect_image(id).await?;
        let labels = image
            .config
            .and_then(|config| config.labels)
            .unwrap_or_default();
        assert_eq!(labels.get("dev.ferrydock.fixture.project"), Some(&project));
        assert!(
            image
                .repo_tags
                .unwrap_or_default()
                .iter()
                .all(|tag| tag.starts_with(&format!("{project}:")))
        );
    }
    let container = docker.inspect_container(&container_id, None).await?;
    let labels = container
        .config
        .and_then(|config| config.labels)
        .unwrap_or_default();
    assert_eq!(labels.get("dev.ferrydock.fixture.project"), Some(&project));
    assert_eq!(container.image.as_deref(), Some(image_id.as_str()));
    let (backend, mut events) = Backend::start();
    let images = inventory(&backend, &mut events, 1).await?;
    let fixture = images
        .iter()
        .find(|image| image.id == image_id)
        .context("Owned image absent")?;
    assert_eq!(fixture.containers.len(), 1);
    assert_eq!(fixture.containers[0].id, container_id);
    assert_eq!(fixture.containers[0].state, "running");
    assert!(fixture.size.is_some());
    assert_eq!(fixture.tags, vec![format!("{project}:main")]);
    assert!(
        remove(&backend, &mut events, &image_id, 1)
            .await?
            .unwrap_err()
            .contains("used by")
    );
    docker::perform(&docker, &container_id, docker::Action::Stop).await?;
    let images = inventory(&backend, &mut events, 2).await?;
    let fixture = images.iter().find(|image| image.id == image_id).unwrap();
    assert_eq!(fixture.containers[0].state, "exited");
    assert!(
        remove(&backend, &mut events, &image_id, 2)
            .await?
            .unwrap_err()
            .contains("stopped containers")
    );
    docker
        .inspect_image(&image_id)
        .await
        .context("Referenced image must remain")?;
    println!(
        "PASS immutable image IDs, tags, sizes, running and stopped references, and removal guards"
    );

    // Docker refuses ID deletion of a multi-tagged image without force.
    assert!(remove(&backend, &mut events, &tagged_id, 3).await?.is_err());
    assert_eq!(
        docker
            .inspect_image(&tagged_id)
            .await?
            .repo_tags
            .unwrap()
            .len(),
        2
    );
    println!("PASS Docker conflict surfaced without force or partial tag removal");

    let results = remove_many(
        &backend,
        &mut events,
        vec![image_id.clone(), tagged_id.clone(), spare_ids[0].clone()],
        4,
    )
    .await?;
    assert_eq!(results.len(), 3);
    assert!(
        results
            .iter()
            .find(|entry| entry.id == spare_ids[0])
            .unwrap()
            .result
            .is_ok()
    );
    assert!(
        results
            .iter()
            .find(|entry| entry.id == image_id)
            .unwrap()
            .result
            .as_ref()
            .unwrap_err()
            .contains("stopped containers")
    );
    assert!(
        results
            .iter()
            .find(|entry| entry.id == tagged_id)
            .unwrap()
            .result
            .is_err()
    );
    docker.inspect_image(&image_id).await?;
    docker.inspect_image(&tagged_id).await?;
    assert!(matches!(
        docker.inspect_image(&spare_ids[0]).await,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        })
    ));
    println!(
        "PASS bulk removal continues after protected/conflicting images and reports each outcome"
    );

    docker.remove_container(&container_id, None).await?;
    let results = remove_many(
        &backend,
        &mut events,
        vec![image_id.clone(), spare_ids[1].clone()],
        5,
    )
    .await?;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|entry| entry.result.is_ok()));
    match docker.inspect_image(&image_id).await {
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {}
        other => bail!("Removed image still exists or unexpected inspect result: {other:?}"),
    }
    let images = inventory(&backend, &mut events, 3).await?;
    assert!(!images.iter().any(|image| image.id == image_id));
    assert!(!images.iter().any(|image| spare_ids.contains(&image.id)));
    assert!(images.iter().any(|image| image.id == tagged_id));
    docker
        .inspect_image("alpine:3.23")
        .await
        .context("Base image must remain")?;
    println!("PASS two-image bulk deletion, inventory reconciliation, other/base images preserved");
    backend.send(Command::CloseImages);
    Ok(())
}
