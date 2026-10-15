//! Read-only network discovery and endpoint details on the Docker runtime.
use crate::docker::Event;
use anyhow::{Context, Result, bail};
use bollard::{
    Docker,
    models::{ContainerSummary, EndpointSettings, Ipam, Network as DockerNetwork},
    query_parameters::{ListContainersOptionsBuilder, ListNetworksOptions},
};
use futures_util::{StreamExt, stream};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub state: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub aliases: Option<Vec<String>>,
    pub dns_names: Option<Vec<String>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Network {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub internal: Option<bool>,
    pub ipv6: Option<bool>,
    pub subnets: Vec<String>,
    pub gateways: Vec<String>,
    pub labels: BTreeMap<String, String>,
    pub members: Arc<[Member]>,
}

impl Network {
    pub fn project(&self) -> Option<&str> {
        self.labels
            .get("com.docker.compose.project")
            .map(String::as_str)
    }

    pub fn matches(&self, filter: &str) -> bool {
        [&self.name, &self.id, &self.driver, &self.scope]
            .into_iter()
            .map(String::as_str)
            .chain(self.project())
            .chain(self.subnets.iter().map(String::as_str))
            .any(|value| value.to_lowercase().contains(filter))
    }
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn address(value: Option<String>, prefix: Option<i64>) -> Option<String> {
    nonempty(value).map(|value| match prefix {
        Some(prefix) => format!("{value}/{prefix}"),
        None => value,
    })
}

fn member(container: &ContainerSummary, endpoint: &EndpointSettings) -> Member {
    let id = container.id.clone().unwrap_or_default();
    Member {
        name: container
            .names
            .as_ref()
            .and_then(|names| names.first())
            .map(|name| name.trim_start_matches('/').to_owned())
            .unwrap_or_else(|| id.chars().take(12).collect()),
        id,
        state: container
            .state
            .map(|state| state.to_string())
            .unwrap_or_else(|| "Unavailable".into()),
        ipv4: address(endpoint.ip_address.clone(), endpoint.ip_prefix_len),
        ipv6: address(
            endpoint.global_ipv6_address.clone(),
            endpoint.global_ipv6_prefix_len,
        ),
        aliases: endpoint.aliases.clone(),
        dns_names: endpoint.dns_names.clone(),
        error: None,
    }
}

pub fn from_summaries(
    networks: Vec<DockerNetwork>,
    containers: Vec<ContainerSummary>,
) -> Result<Vec<Network>> {
    if networks.len() > 20_000 || containers.len() > 50_000 {
        bail!("Network inventory exceeds FerryDock's limit (20,000 networks / 50,000 containers)");
    }
    let mut references: BTreeMap<String, Vec<Member>> = BTreeMap::new();
    let names: BTreeMap<_, _> = networks
        .iter()
        .filter_map(|network| Some((network.name.as_deref()?, network.id.as_deref()?)))
        .collect();
    let mut count = 0;
    for container in &containers {
        let Some(endpoints) = container
            .network_settings
            .as_ref()
            .and_then(|settings| settings.networks.as_ref())
        else {
            continue;
        };
        for (name, endpoint) in endpoints {
            let id = endpoint
                .network_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .or_else(|| names.get(name.as_str()).copied());
            let Some(id) = id else {
                continue;
            };
            count += 1;
            if count > 100_000 {
                bail!("Network inventory exceeds FerryDock's limit (100,000 container references)");
            }
            if container.id.as_deref().is_none_or(str::is_empty) {
                bail!("Docker returned an unidentified network container");
            }
            references
                .entry(id.to_owned())
                .or_default()
                .push(member(container, endpoint));
        }
    }
    let mut items = Vec::with_capacity(networks.len());
    for network in networks {
        let id = nonempty(network.id).context("Docker returned a network without an ID")?;
        let name = nonempty(network.name).unwrap_or_else(|| id.chars().take(12).collect());
        let Ipam { config, .. } = network.ipam.unwrap_or_default();
        let config = config.unwrap_or_default();
        let mut members = references.remove(&id).unwrap_or_default();
        members.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        members.dedup_by(|a, b| a.id == b.id);
        items.push(Network {
            id,
            name,
            driver: nonempty(network.driver).unwrap_or_else(|| "Unavailable".into()),
            scope: nonempty(network.scope).unwrap_or_else(|| "Unavailable".into()),
            internal: network.internal,
            ipv6: network.enable_ipv6,
            subnets: config
                .iter()
                .filter_map(|item| nonempty(item.subnet.clone()))
                .collect(),
            gateways: config
                .iter()
                .filter_map(|item| nonempty(item.gateway.clone()))
                .collect(),
            labels: network.labels.unwrap_or_default().into_iter().collect(),
            members: members.into(),
        });
    }
    items.sort_by_cached_key(|network| (network.name.to_lowercase(), network.id.clone()));
    Ok(items)
}

async fn containers(docker: &Docker) -> Result<Vec<ContainerSummary>> {
    Ok(docker
        .list_containers(Some(
            ListContainersOptionsBuilder::default().all(true).build(),
        ))
        .await?)
}

pub async fn snapshot(docker: &Docker) -> Result<Vec<Network>> {
    time::timeout(Duration::from_secs(15), async {
        let (networks, containers) = tokio::try_join!(
            async {
                Ok::<_, anyhow::Error>(docker.list_networks(None::<ListNetworksOptions>).await?)
            },
            containers(docker),
        )?;
        from_summaries(networks, containers)
    })
    .await
    .context("Network discovery timed out")?
}

pub async fn details(docker: &Docker, id: &str) -> Result<Network> {
    time::timeout(Duration::from_secs(15), async {
        let (inspected, containers) = tokio::try_join!(
            async { Ok::<_, anyhow::Error>(docker.inspect_network(id, None).await?) },
            containers(docker),
        )?;
        let summary = DockerNetwork {
            id: inspected.id,
            name: inspected.name,
            driver: inspected.driver,
            scope: inspected.scope,
            internal: inspected.internal,
            enable_ipv6: inspected.enable_ipv6,
            ipam: inspected.ipam,
            labels: inspected.labels,
            ..Default::default()
        };
        let mut network = from_summaries(vec![summary], containers)?
            .pop()
            .context("Network disappeared")?;
        let mut members = network.members.to_vec();
        let mut known: HashSet<String> = members.iter().map(|member| member.id.clone()).collect();
        // Overlay endpoints may be returned without a local container summary.
        for (id, endpoint) in inspected.containers.unwrap_or_default() {
            if known.insert(id.clone()) {
                if members.len() >= 50_000 {
                    bail!("Network exceeds FerryDock's limit (50,000 endpoints)");
                }
                members.push(Member {
                    name: nonempty(endpoint.name).unwrap_or_else(|| id.chars().take(12).collect()),
                    id,
                    state: "Unavailable".into(),
                    ipv4: nonempty(endpoint.ipv4_address),
                    ipv6: nonempty(endpoint.ipv6_address),
                    aliases: None,
                    dns_names: None,
                    error: None,
                });
            }
        }
        let name = &network.name;
        let network_id = &network.id;
        let mut members = stream::iter(members.into_iter().map(|mut member| async move {
            let result = time::timeout(
                Duration::from_secs(3),
                docker.inspect_container(&member.id, None),
            )
            .await;
            match result {
                Ok(Ok(container)) => {
                    member.state = container
                        .state
                        .and_then(|state| state.status)
                        .map(|status| status.to_string())
                        .unwrap_or_else(|| "Unavailable".into());
                    let endpoint = container
                        .network_settings
                        .and_then(|settings| settings.networks)
                        .and_then(|networks| {
                            networks
                                .into_iter()
                                .find(|(key, value)| {
                                    value.network_id.as_ref() == Some(network_id) || key == name
                                })
                                .map(|(_, value)| value)
                        });
                    if let Some(endpoint) = endpoint {
                        member.ipv4 = address(endpoint.ip_address, endpoint.ip_prefix_len);
                        member.ipv6 = address(
                            endpoint.global_ipv6_address,
                            endpoint.global_ipv6_prefix_len,
                        );
                        member.aliases = endpoint.aliases;
                        member.dns_names = endpoint.dns_names;
                    } else {
                        member.ipv4 = None;
                        member.ipv6 = None;
                        member.error =
                            Some("Network attachment changed; refresh to check membership".into());
                    }
                }
                Ok(Err(error)) => {
                    member.error = Some(format!("Endpoint details unavailable: {error}"))
                }
                Err(_) => member.error = Some("Endpoint details timed out".into()),
            }
            member
        }))
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
        members.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        network.members = members.into();
        Ok(network)
    })
    .await
    .context("Network details timed out; refresh to try again")?
}

/// Inventory and selected-network requests use separate subscriptions. Changing
/// a request cancels its in-flight reads; hidden workspaces do not keep polling.
pub(crate) async fn follow(
    docker: Docker,
    mut requests: watch::Receiver<Option<(Option<String>, u64)>>,
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
) {
    let mut interval = time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut immediate = true;
    loop {
        if !immediate {
            tokio::select! {
                _ = cancel.cancelled() => return,
                changed = requests.changed() => if changed.is_err() { return; },
                _ = interval.tick() => {},
            }
        }
        immediate = false;
        let request = requests.borrow_and_update().clone();
        let Some((id, generation)) = request else {
            continue;
        };
        let read = async {
            match &id {
                Some(id) => Event::NetworkDetails {
                    id: id.clone(),
                    generation,
                    result: details(&docker, id)
                        .await
                        .map_err(|error| format!("{error:#}")),
                },
                None => Event::Networks {
                    generation,
                    result: snapshot(&docker)
                        .await
                        .map_err(|error| format!("{error:#}")),
                },
            }
        };
        let event = tokio::select! {
            _ = cancel.cancelled() => return,
            changed = requests.changed() => { if changed.is_err() { return; } immediate = true; continue; },
            event = read => event,
        };
        tokio::select! {
            _ = cancel.cancelled() => return,
            changed = requests.changed() => { if changed.is_err() { return; } immediate = true; },
            _ = events.send(event) => {},
        }
        interval.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_members_include_stopped_and_missing_addresses_are_unavailable() {
        let networks = serde_json::from_value(serde_json::json!([{"Id":"network-id","Name":"demo_default","Driver":"bridge","Labels":{"com.docker.compose.project":"Demo"},"IPAM":{"Config":[{"Subnet":"172.30.0.0/16","Gateway":"172.30.0.1"}]}}])).unwrap();
        let containers = serde_json::from_value(serde_json::json!([
            {"Id":"running","Names":["/api"],"State":"running","NetworkSettings":{"Networks":{"demo_default":{"NetworkID":"network-id","IPAddress":"172.30.0.2","IPPrefixLen":16}}}},
            {"Id":"stopped","Names":["/database"],"State":"exited","NetworkSettings":{"Networks":{"demo_default":{"NetworkID":"","IPAddress":"","Aliases":["db"]}}}}
        ])).unwrap();
        let items = from_summaries(networks, containers).unwrap();
        assert_eq!(items[0].members.len(), 2);
        assert_eq!(items[0].members[0].ipv4.as_deref(), Some("172.30.0.2/16"));
        assert_eq!(items[0].members[1].ipv4, None);
        assert_eq!(items[0].members[1].state, "exited");
        assert!(items[0].matches("demo"));
        assert!(items[0].matches("172.30"));
        assert_eq!(items[0].members[0].aliases, None);
    }
}
