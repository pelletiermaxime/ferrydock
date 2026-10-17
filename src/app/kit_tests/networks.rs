use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::networks::{Member, Network};
    let started = Instant::now();
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Network fixture".into(),
        endpoint: "unix:///fixture.sock".into(),
    }));
    h.deliver(Event::Snapshot(vec![
        container("api", "fixture-api", "running", Some("demo")),
        container("db", "fixture-db", "exited", Some("demo")),
    ]));
    let selected = h.observe(|view| view.containers.selected.clone());
    h.key("enter");
    let terminal_generation = h.observe(|view| view.terminal.generation);
    h.deliver(Event::TerminalReady {
        generation: terminal_generation,
    });
    h.deliver(Event::TerminalData {
        generation: terminal_generation,
        data: b"NETWORK_SESSION_MARKER\r\n".to_vec(),
    });
    assert!(h.observe(|view| view.terminal.view.is_some()));
    h.drain_commands();
    h.click("0-3");
    assert!(h.observe(|view| view.networks_open));
    assert!(h.observe(
        |view| view.terminal.view.is_some() && view.terminal.generation == terminal_generation
    ));
    assert_eq!(h.observe(|view| view.containers.selected.clone()), selected);
    let generation = h.observe(|view| view.networks.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command,Command::Networks { generation: current } if *current == generation)));
    let members: Arc<[Member]> = vec![
        Member {
            id: "api".into(),
            name: "fixture-api".into(),
            state: "running".into(),
            ipv4: Some("172.30.0.2/16".into()),
            ipv6: Some("fd00::2/64".into()),
            aliases: Some(vec!["api".into(), "backend".into()]),
            dns_names: Some(vec!["fixture-api".into()]),
            error: None,
        },
        Member {
            id: "db".into(),
            name: "fixture-db".into(),
            state: "exited".into(),
            ipv4: None,
            ipv6: None,
            aliases: Some(vec!["database".into()]),
            dns_names: None,
            error: None,
        },
    ]
    .into();
    let network = Network {
        id: "demo-network".into(),
        name: "demo_default".into(),
        driver: "bridge".into(),
        scope: "local".into(),
        internal: Some(false),
        ipv6: Some(true),
        subnets: vec!["172.30.0.0/16".into(), "fd00::/64".into()],
        gateways: vec!["172.30.0.1".into(), "fd00::1".into()],
        labels: [("com.docker.compose.project".into(), "demo".into())].into(),
        members,
    };
    let other = Network {
        id: "other-network".into(),
        name: "other_default".into(),
        members: Arc::from([]),
        ..network.clone()
    };
    h.deliver(Event::Networks {
        generation,
        result: Ok(vec![network.clone(), other.clone()]),
    });
    let detail_generation = h.observe(|view| view.networks.detail_generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command,Command::NetworkDetails {id,generation} if id == "demo-network" && *generation == detail_generation)));
    h.checkpoint("networks-loading-details");
    h.deliver(Event::NetworkDetails {
        id: network.id.clone(),
        generation: detail_generation,
        result: Ok(network.clone()),
    });
    h.checkpoint("networks-endpoints");
    h.click("network-copy-api");
    h.cx.update_window(h.window.into(), |_, _, cx| {
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        assert!(
            text.contains("172.30.0.2/16")
                && text.contains("fd00::2/64")
                && text.contains("backend")
        );
    })
    .unwrap();
    h.key("ctrl-f");
    h.type_text("other");
    assert_eq!(h.observe(|view| view.networks.visible.len()), 1);
    h.checkpoint("networks-search");
    h.key("ctrl-a");
    h.type_text("no-such-network");
    assert!(h.observe(|view| view.networks.visible.is_empty()));
    h.checkpoint("networks-empty-search");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.key("down");
    let other_generation = h.observe(|view| view.networks.detail_generation);
    assert_eq!(
        h.observe(|view| view.networks.selected.clone()),
        Some(other.id.clone())
    );
    h.deliver(Event::NetworkDetails {
        id: network.id.clone(),
        generation: detail_generation,
        result: Ok(network.clone()),
    });
    assert!(h.observe(|view| view.networks.details.is_none()));
    h.deliver(Event::NetworkDetails {
        id: other.id.clone(),
        generation: other_generation,
        result: Err("Fixture endpoint read failed".into()),
    });
    h.checkpoint("networks-detail-error");
    h.key("ctrl-r");
    let generation = h.observe(|view| view.networks.generation);
    let current_details = h.observe(|view| view.networks.detail_generation);
    h.deliver(Event::Networks {
        generation,
        result: Ok(vec![network.clone(), other.clone()]),
    });
    h.deliver(Event::NetworkDetails {
        id: other.id.clone(),
        generation: current_details,
        result: Ok(other.clone()),
    });
    assert_eq!(
        h.observe(|view| view.networks.selected.clone()),
        Some(other.id.clone())
    );
    h.checkpoint("networks-empty-endpoints");
    h.key("up");
    let detail_generation = h.observe(|view| view.networks.detail_generation);
    h.deliver(Event::NetworkDetails {
        id: network.id.clone(),
        generation: detail_generation,
        result: Ok(network.clone()),
    });
    h.click("network-open-db");
    assert!(!h.observe(|view| view.networks_open));
    assert_eq!(
        h.observe(|view| view.containers.selected.clone()),
        Some("db".into())
    );
    assert!(!h.observe(|view| view.containers.running_only));
    assert!(
        h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::CloseNetworks))
    );
    h.checkpoint("networks-open-stopped-container");
    h.click("container-more");
    h.click_popup_item(3);
    assert!(h.observe(|view| view.networks_open));
    let generation = h.observe(|view| view.networks.generation);
    h.deliver(Event::Networks {
        generation,
        result: Ok(vec![network.clone(), other.clone()]),
    });
    assert_eq!(h.observe(|view| view.networks.visible.len()), 1);
    let detail_generation = h.observe(|view| view.networks.detail_generation);
    h.deliver(Event::NetworkDetails {
        id: network.id.clone(),
        generation: detail_generation,
        result: Ok(network.clone()),
    });
    h.checkpoint("networks-for-container");
    h.click("network-show-all");
    assert_eq!(h.observe(|view| view.networks.visible.len()), 2);
    // Refresh preserves selection but rejects results from earlier requests.
    h.key("ctrl-r");
    h.deliver(Event::NetworkDetails {
        id: network.id.clone(),
        generation: detail_generation,
        result: Ok(network.clone()),
    });
    assert!(h.observe(|view| view.networks.details.is_none()));
    let generation = h.observe(|view| view.networks.generation);
    let many = (0..10_000)
        .map(|index| Network {
            id: format!("large-{index}"),
            name: format!("network-{index:05}"),
            members: Arc::from([]),
            ..network.clone()
        })
        .collect();
    h.deliver(Event::Networks {
        generation,
        result: Ok(many),
    });
    h.checkpoint("networks-large-inventory");
    assert!(h.observe(|view| view.networks.rows_rendered.get()) < 80);
    let selected = h.observe(|view| view.networks.selected.clone()).unwrap();
    let many_members = (0..2_000)
        .map(|index| Member {
            id: format!("endpoint-{index}"),
            name: format!("container-{index:05}"),
            ..network.members[0].clone()
        })
        .collect::<Vec<_>>();
    let large = Network {
        id: selected.clone(),
        name: "Large network".into(),
        members: many_members.into(),
        ..network.clone()
    };
    let details = h.observe(|view| view.networks.detail_generation);
    h.deliver(Event::NetworkDetails {
        id: selected.clone(),
        generation: details,
        result: Ok(large),
    });
    h.checkpoint("networks-large-endpoints");
    assert!(h.observe(|view| view.networks.members_rendered.get()) < 80);
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("networks-light");
    h.deliver(Event::Disconnected("Fixture Docker disconnected".into()));
    h.deliver(Event::NetworkDetails {
        id: selected,
        generation: details,
        result: Ok(network.clone()),
    });
    assert!(h.observe(|view| view.networks.details.is_none() && view.networks.items.is_empty()));
    h.checkpoint("networks-disconnected");
    h.deliver(Event::Connected(Connection {
        name: "Network fixture".into(),
        endpoint: "unix:///fixture.sock".into(),
    }));
    let generation = h.observe(|view| view.networks.generation);
    h.deliver(Event::Networks {
        generation,
        result: Ok(vec![network.clone()]),
    });
    let details = h.observe(|view| view.networks.detail_generation);
    h.deliver(Event::NetworkDetails {
        id: network.id.clone(),
        generation: details,
        result: Ok(network),
    });
    h.checkpoint("networks-reconnected");
    h.deliver(Event::Connected(Connection {
        name: "Different context".into(),
        endpoint: "unix:///different.sock".into(),
    }));
    assert!(h.observe(|view| view.networks.selected.is_none()
        && view.networks.details.is_none()
        && view.networks.container_filter.is_none()));
    let generation = h.observe(|view| view.networks.generation);
    h.deliver(Event::Networks {
        generation,
        result: Err("Fixture inventory unavailable".into()),
    });
    h.checkpoint("networks-inventory-error");
    let report = json!({"docker_worker_started":false,
        "render_and_flow_seconds":started.elapsed().as_secs_f64(),
        "checks":["sidebar session preservation","search and keyboard selection","configured stopped endpoints and IPv4/IPv6 CIDR","aliases/DNS copy in virtual clipboard","detail failure/retry","refresh/selection/disconnect stale guards","open stopped container","container-scoped network navigation","10000 networks and 2000 virtualized endpoints","Light theme","reconnection"]});
    h.finish(report);
}
