//! Production worker/daemon proof, restricted to script-owned volumes and container.
use anyhow::{Context, Result, bail};
use ferrydock::{
    docker::{self, Backend, Command, Event},
    volumes::Volume,
};
use std::{env, time::Duration};
use tokio::{sync::mpsc, time};

async fn inventory(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    generation: u64,
) -> Result<Vec<Volume>> {
    backend.send(Command::Volumes { generation });
    time::timeout(Duration::from_secs(25), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::Volumes {
                    generation: current,
                    result,
                } if current == generation => return result.map_err(anyhow::Error::msg),
                Event::VolumeSizes { .. } => bail!("Normal discovery unexpectedly gathered sizes"),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Volume discovery timed out")?
}

async fn remove(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    name: &str,
    operation: u64,
) -> Result<Result<(), String>> {
    backend.send(Command::RemoveVolume {
        name: name.into(),
        operation,
    });
    time::timeout(Duration::from_secs(50), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::VolumeRemoved {
                    name: current,
                    operation: current_operation,
                    result,
                } if current == name && current_operation == operation => return Ok(result),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Volume removal timed out")?
}

async fn remove_many(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    names: Vec<String>,
    operation: u64,
) -> Result<Vec<ferrydock::volumes::RemovalResult>> {
    backend.send(Command::RemoveVolumes { names, operation });
    time::timeout(Duration::from_secs(120), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::VolumesRemoved {
                    operation: current,
                    results,
                } if current == operation => return Ok(results),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Volume bulk removal timed out")?
}

#[tokio::test]
#[ignore = "Uses only fixtures from scripts/verify-volumes.sh"]
async fn volume_worker_discovery_and_guarded_removal() -> Result<()> {
    let project = env::var("FERRYDOCK_VOLUME_TEST_PROJECT")?;
    let container_id = env::var("FERRYDOCK_VOLUME_TEST_CONTAINER")?;
    let endpoint = env::var("FERRYDOCK_VOLUME_TEST_ENDPOINT")?;
    let used = format!("{project}-used");
    let unused = format!("{project}-unused");
    let preserved = format!("{project}-preserved");
    let bulk_a = format!("{project}-bulk-a");
    let bulk_b = format!("{project}-bulk-b");
    let missing = format!("{project}-missing");
    let connection = docker::resolve_connection().await?;
    assert_eq!(connection.endpoint, endpoint);
    let docker = docker::connect(&connection).await?;
    for name in [&used, &unused, &preserved, &bulk_a, &bulk_b] {
        let volume = docker.inspect_volume(name).await?;
        assert_eq!(
            volume.labels.get("dev.ferrydock.fixture.project"),
            Some(&project)
        );
    }
    let container = docker.inspect_container(&container_id, None).await?;
    assert_eq!(
        container
            .config
            .and_then(|config| config.labels)
            .unwrap_or_default()
            .get("dev.ferrydock.fixture.project"),
        Some(&project)
    );
    let (backend, mut events) = Backend::start();
    let volumes = inventory(&backend, &mut events, 1).await?;
    let volume = volumes
        .iter()
        .find(|volume| volume.name == used)
        .context("Owned volume absent")?;
    assert_eq!(volume.driver, "local");
    assert_eq!(volume.project(), Some(project.as_str()));
    assert!(!volume.mountpoint.is_empty());
    assert_eq!(volume.containers.len(), 1);
    assert_eq!(volume.containers[0].id, container_id);
    assert_eq!(volume.containers[0].state, "running");
    backend.send(Command::GatherVolumeSizes { request: 77 });
    let sizes = time::timeout(Duration::from_secs(125), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::VolumeSizes {
                    request: 77,
                    result,
                } => return result.map_err(anyhow::Error::msg),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Size gathering timed out")??;
    let measurement = sizes
        .iter()
        .find(|size| size.name == used)
        .context("Owned measurement absent")?;
    assert!(
        measurement.matches(volume),
        "Measurement must identify the discovered volume"
    );
    assert!(
        measurement
            .size
            .context("Size unavailable for owned local volume")?
            >= 3 * 1_048_576
    );
    eprintln!("Owned volume size: {} bytes", measurement.size.unwrap());
    inventory(&backend, &mut events, 78).await?;
    assert!(
        remove(&backend, &mut events, &used, 1)
            .await?
            .unwrap_err()
            .contains("used by")
    );
    docker::perform(&docker, &container_id, docker::Action::Stop).await?;
    time::timeout(Duration::from_secs(20), async {
        let mut generation = 2;
        loop {
            let volumes = inventory(&backend, &mut events, generation).await?;
            if volumes
                .iter()
                .find(|volume| volume.name == used)
                .context("Owned volume absent")?
                .containers
                .iter()
                .any(|container| container.id == container_id && container.state == "exited")
            {
                return Ok::<_, anyhow::Error>(());
            }
            generation += 1;
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("Stopped volume reference not observed")??;
    assert!(
        remove(&backend, &mut events, &used, 2)
            .await?
            .unwrap_err()
            .contains("stopped containers")
    );
    docker
        .inspect_volume(&used)
        .await
        .context("Referenced volume must remain")?;
    println!("PASS discovery/configuration and running/stopped removal guards");
    let results = remove_many(
        &backend,
        &mut events,
        vec![used.clone(), missing.clone(), unused.clone()],
        3,
    )
    .await?;
    assert_eq!(results.len(), 3);
    assert!(
        results
            .iter()
            .find(|entry| entry.name == used)
            .unwrap()
            .result
            .as_ref()
            .unwrap_err()
            .contains("stopped containers")
    );
    assert!(
        results
            .iter()
            .find(|entry| entry.name == missing)
            .unwrap()
            .result
            .is_err()
    );
    assert!(
        results
            .iter()
            .find(|entry| entry.name == unused)
            .unwrap()
            .result
            .is_ok()
    );
    assert!(matches!(
        docker.inspect_volume(&unused).await,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        })
    ));
    let volumes = inventory(&backend, &mut events, 100).await?;
    assert!(!volumes.iter().any(|volume| volume.name == unused));
    assert!(volumes.iter().any(|volume| volume.name == preserved));
    assert!(volumes.iter().any(|volume| volume.name == used));
    println!(
        "PASS partial batch keeps referenced/missing targets, deletes unused volume and reconciles inventory"
    );
    docker.remove_container(&container_id, None).await?;
    let results = remove_many(&backend, &mut events, vec![used.clone(), bulk_a.clone()], 4).await?;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|entry| entry.result.is_ok()));
    remove(&backend, &mut events, &bulk_b, 5)
        .await?
        .map_err(anyhow::Error::msg)?;
    let volumes = inventory(&backend, &mut events, 200).await?;
    assert!(
        !volumes
            .iter()
            .any(|volume| [&used, &unused, &bulk_a, &bulk_b].contains(&&volume.name))
    );
    assert!(matches!(
        docker.inspect_volume(&used).await,
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        })
    ));
    assert!(remove(&backend, &mut events, &used, 6).await?.is_err());
    docker.inspect_volume(&preserved).await?;
    backend.send(Command::CloseVolumes);
    println!("PASS successful two-volume batch, single deletion and missing-volume error");
    Ok(())
}
