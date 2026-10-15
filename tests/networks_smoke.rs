//! Production worker reads, restricted mutations and owned fixture validation.
use anyhow::{Context, Result, bail};
use ferrydock::{
    docker::{self, Backend, Command, Event},
    networks::Network,
};
use std::{env, time::Duration};
use tokio::{sync::mpsc, time};

async fn inventory(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    generation: u64,
) -> Result<Vec<Network>> {
    backend.send(Command::Networks { generation });
    time::timeout(Duration::from_secs(25), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::Networks {
                    generation: current,
                    result,
                } if current == generation => return result.map_err(anyhow::Error::msg),
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Network inventory timed out")?
}

async fn details(
    backend: &Backend,
    events: &mut mpsc::Receiver<Event>,
    id: &str,
    generation: u64,
) -> Result<Network> {
    backend.send(Command::NetworkDetails {
        id: id.into(),
        generation,
    });
    time::timeout(Duration::from_secs(25), async {
        loop {
            match events.recv().await.context("Worker ended")? {
                Event::NetworkDetails {
                    id: current,
                    generation: active,
                    result,
                } if current == id && active == generation => {
                    return result.map_err(anyhow::Error::msg);
                }
                Event::Disconnected(error) => bail!("Docker disconnected: {error}"),
                _ => {}
            }
        }
    })
    .await
    .context("Network details timed out")?
}

#[tokio::test]
#[ignore = "Uses only fixtures from scripts/verify-networks.sh"]
async fn network_worker_inventory_addresses_aliases_and_stopped_members() -> Result<()> {
    let project = env::var("FERRYDOCK_NETWORK_TEST_PROJECT")?;
    let endpoint = env::var("FERRYDOCK_NETWORK_TEST_ENDPOINT")?;
    let api = env::var("FERRYDOCK_NETWORK_TEST_API")?;
    let db = env::var("FERRYDOCK_NETWORK_TEST_DB")?;
    let connection = docker::resolve_connection().await?;
    assert_eq!(connection.endpoint, endpoint);
    let docker = docker::connect(&connection).await?;
    for id in [&api, &db] {
        let inspected = docker.inspect_container(id, None).await?;
        assert_eq!(
            inspected
                .config
                .and_then(|config| config.labels)
                .unwrap_or_default()
                .get("dev.ferrydock.fixture.project"),
            Some(&project)
        );
    }
    let (backend, mut events) = Backend::start();
    let items = inventory(&backend, &mut events, 1).await?;
    let network = items
        .iter()
        .find(|network| network.name == format!("{project}-attached"))
        .context("Owned network absent")?;
    assert_eq!(
        network.labels.get("dev.ferrydock.fixture.project"),
        Some(&project)
    );
    assert_eq!(network.driver, "bridge");
    assert_eq!(
        network.members.len(),
        2,
        "Running and stopped containers must be present"
    );
    let inspected = details(&backend, &mut events, &network.id, 2).await?;
    let running = inspected
        .members
        .iter()
        .find(|member| member.id == api)
        .context("Running member absent")?;
    let stopped = inspected
        .members
        .iter()
        .find(|member| member.id == db)
        .context("Stopped member absent")?;
    assert_eq!(running.state, "running");
    assert!(
        running
            .ipv4
            .as_ref()
            .is_some_and(|value| value.contains('/'))
    );
    assert!(
        running
            .aliases
            .as_ref()
            .is_some_and(|aliases| aliases.iter().any(|alias| alias == "backend"))
    );
    assert!(running.error.is_none(), "{:?}", running.error);
    assert_eq!(stopped.state, "exited");
    assert_eq!(stopped.ipv4, None);
    assert!(
        stopped
            .aliases
            .as_ref()
            .is_some_and(|aliases| aliases.iter().any(|alias| alias == "database"))
    );
    assert!(stopped.error.is_none(), "{:?}", stopped.error);
    assert!(!inspected.subnets.is_empty());
    assert!(!inspected.gateways.is_empty());
    let empty = items
        .iter()
        .find(|network| network.name == format!("{project}-empty"))
        .context("Empty network absent")?;
    assert!(
        details(&backend, &mut events, &empty.id, 3)
            .await?
            .members
            .is_empty()
    );
    backend.send(Command::CloseNetworks);
    // The fixture owns this endpoint. Disconnect it to prove fresh membership.
    docker
        .disconnect_network(
            &network.id,
            bollard::models::NetworkDisconnectRequest {
                container: api.clone(),
                force: Some(false),
            },
        )
        .await?;
    let items = inventory(&backend, &mut events, 4).await?;
    let network = items
        .iter()
        .find(|item| item.id == network.id)
        .context("Network disappeared")?;
    assert_eq!(network.members.len(), 1);
    assert_eq!(network.members[0].id, db);
    assert_eq!(
        details(&backend, &mut events, &network.id, 5)
            .await?
            .members
            .len(),
        1
    );
    eprintln!(
        "Verified network {}: running IPv4/aliases, stopped aliases without IP, empty network, and disconnect reconciliation",
        network.name
    );
    Ok(())
}
