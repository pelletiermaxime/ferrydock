use super::*;

pub(super) fn verify_volumes_workspace(h: &mut Harness) {
    use crate::volumes::{Volume, VolumeContainer};
    h.deliver(Event::Connected(crate::docker::Connection {
        name: "headless-fixture".into(),
        endpoint: "unix:///headless-fixture.sock".into(),
    }));
    h.settle_ui(5500);
    h.click("0-0");
    h.click("row-fixture-probe-running");
    let unused = Volume {
        name: "demo-cache".into(),
        driver: "local".into(),
        mountpoint: "/var/lib/docker/volumes/demo-cache/_data".into(),
        created: Some("2026-10-13T10:00:00-04:00".into()),
        scope: Some("local".into()),
        labels: [("com.docker.compose.project".into(), "sample-project".into())].into(),
        options: Default::default(),
        size: None,
        containers: vec![].into(),
    };
    let used = Volume {
        name: "demo-database".into(),
        containers: vec![VolumeContainer {
            id: "fixture-probe-stopped".into(),
            name: "demo-database-stopped".into(),
            state: "exited".into(),
        }]
        .into(),
        ..unused.clone()
    };
    let remote = Volume {
        name: "remote-data".into(),
        driver: "nfs".into(),
        labels: Default::default(),
        options: [("type".into(), "nfs".into())].into(),
        ..unused.clone()
    };
    let inventory = vec![unused.clone(), used.clone(), remote.clone()];
    let session = h.observe(|v| {
        (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation,
        )
    });
    h.drain_commands();
    h.click("0-2"); // Workspace -> Volumes.
    assert!(h.observe(|v| v.volumes_open && !v.images_open));
    let mut generation = h.observe(|v| v.volumes.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Volumes { generation: current } if *current == generation)));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.checkpoint("volumes-inventory");
    verify_volume_sizes(h, &inventory);
    verify_volume_sorting_and_selection(h, &inventory);
    generation = h.observe(|v| v.volumes.generation);
    h.click("volume-row-demo-database");
    h.click("volume-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolume { .. }))
    );
    h.checkpoint("volumes-stopped-reference");
    for (query, count) in [
        ("DEMO-CACHE", 1),
        ("NFS", 1),
        ("SAMPLE-PROJECT", 2),
        ("no-such-volume", 0),
    ] {
        h.key("ctrl-f");
        h.key("ctrl-a");
        h.type_text(query);
        assert_eq!(h.observe(|v| v.volumes.visible.len()), count);
    }
    h.checkpoint("volumes-no-matches");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("volume-row-demo-cache");
    h.click("volume-remove");
    assert_eq!(
        h.observe(|v| v.volumes.confirmation.clone()),
        Some(vec![unused.name.clone()])
    );
    h.checkpoint("volumes-removal-review");
    h.click("volume-cancel-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.click("volume-remove");
    h.key("escape");
    h.settle_ui(300);
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolume { .. }))
    );
    // A stopped reference appearing during review invalidates the open dialog.
    h.click("volume-remove");
    let mut now_used = unused.clone();
    now_used.containers = used.containers.clone();
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![now_used, used.clone(), remote.clone()]),
    });
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveVolume { name, operation } if name == &unused.name => Some(*operation),
            _ => None,
        })
        .unwrap();
    h.click("volume-remove");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolume { .. }))
    );
    h.checkpoint("volumes-removing");
    h.click("0-0");
    h.drain_commands();
    h.click("primary-action");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Action { .. }))
    );
    h.click("0-2");
    h.deliver(Event::VolumeRemoved {
        name: unused.name.clone(),
        operation: operation + 1,
        result: Ok(()),
    });
    assert!(h.observe(|v| v.volumes.busy.is_some()));
    h.deliver(Event::VolumeRemoved {
        name: unused.name.clone(),
        operation,
        result: Err("Fixture volume conflict".into()),
    });
    assert!(h.observe(|v| v.volumes.busy.is_none()));
    assert_eq!(
        h.observe(|v| v.volumes.selected.clone()),
        Some(unused.name.clone())
    );
    h.checkpoint("volumes-removal-failed");
    h.dismiss_image_feedback();
    generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h.observe(|v| v.volumes.busy.as_ref().unwrap().1);
    let old_generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::VolumeRemoved {
        name: unused.name.clone(),
        operation,
        result: Ok(()),
    });
    h.deliver(Event::Volumes {
        generation: old_generation,
        result: Ok(inventory.clone()),
    });
    assert!(!h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == unused.name)
    }));
    generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![used.clone(), remote.clone()]),
    });
    h.checkpoint("volumes-removed");
    h.dismiss_image_feedback();
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    assert!(h.observe(|v| v.volumes_open && !v.settings_open));
    h.checkpoint("volumes-light");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    let many = (0..10_000)
        .map(|index| Volume {
            name: format!("fixture-volume-{index:05}"),
            ..unused.clone()
        })
        .collect();
    h.deliver(Event::Volumes {
        generation,
        result: Ok(many),
    });
    h.checkpoint("volumes-large-inventory");
    let selection = h.observe(|v| v.volumes.selection.clone());
    h.drag_list_scrollbar("volumes-viewport", true);
    assert!(h.observe(|v| v.volumes.scroll.0.borrow().base_handle.offset().y < px(-1000.0)));
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selection);
    h.checkpoint("volumes-scrollbar-dragged");
    h.drag_list_scrollbar("volumes-viewport", false);
    assert!(h.observe(|v| v.volumes.scroll.0.borrow().base_handle.offset().y > px(-100.0)));
    assert!(h.observe(|v| v.volumes.rows_rendered.get()) < 40);
    h.key("ctrl-f");
    h.type_text("fixture-volume-09999");
    h.key("escape");
    h.key("down");
    assert_eq!(h.observe(|v| v.volumes.visible.len()), 1);
    assert_eq!(
        h.observe(|v| v.volumes.selected.clone()),
        Some("fixture-volume-09999".into())
    );
    h.checkpoint("volumes-large-search");
    h.click("0-0");
    assert!(!h.observe(|v| v.volumes_open));
    assert_eq!(
        session,
        h.observe(|v| (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation
        ))
    );
    assert!(
        h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::CloseVolumes))
    );
    h.click("0-2");
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-row-demo-database");
    h.click("volume-container-fixture-probe-stopped");
    assert!(!h.observe(|v| v.volumes_open));
    assert!(!h.observe(|v| v.containers.running_only));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        Some("fixture-probe-stopped".into())
    );
    h.checkpoint("volumes-reference-navigation");
    h.click("0-2");
    h.drain_commands();
    h.key("ctrl-r");
    generation = h.observe(|v| v.volumes.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Volumes { generation: current } if *current == generation)));
    h.deliver(Event::Volumes {
        generation,
        result: Err("Fixture volume inventory failure".into()),
    });
    h.checkpoint("volumes-discovery-failed");
    h.click("volume-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    let measurement = crate::volumes::VolumeSize {
        name: unused.name.clone(),
        driver: unused.driver.clone(),
        mountpoint: unused.mountpoint.clone(),
        created: unused.created.clone(),
        size: Some(3 * 1_048_576),
    };
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(vec![measurement.clone()]),
    });
    h.click("volume-gather-sizes");
    let interrupted_request = h.observe(|v| v.volumes.gathering.unwrap());
    h.click("volume-row-demo-cache");
    h.ctrl_click("volume-row-remote-data");
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h.observe(|v| v.volumes.busy.as_ref().unwrap().1);
    h.deliver(Event::Disconnected("Fixture Docker disconnect".into()));
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory),
    });
    h.deliver(Event::VolumesRemoved {
        operation,
        results: vec![
            crate::volumes::RemovalResult {
                name: unused.name.clone(),
                result: Ok(()),
            },
            crate::volumes::RemovalResult {
                name: remote.name.clone(),
                result: Ok(()),
            },
        ],
    });
    assert!(!h.observe(|v| v.volumes.loaded));
    assert!(h.observe(|v| v.volumes.busy.is_none()));
    assert!(h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == unused.name)
    }));
    h.deliver(Event::VolumeSizes {
        request: interrupted_request,
        result: Ok(vec![]),
    });
    assert!(h.observe(|v| v.volumes.gathering.is_none()));
    assert_eq!(
        h.observe(|v| v
            .volumes
            .items
            .iter()
            .find(|volume| volume.name == unused.name)
            .unwrap()
            .size),
        measurement.size
    );
    h.drain_commands();
    h.click("volume-gather-sizes");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::GatherVolumeSizes { .. }))
    );
    h.deliver(Event::Connected(Connection {
        name: "headless-fixture".into(),
        endpoint: "unix:///headless-fixture.sock".into(),
    }));
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![unused.clone(), used.clone(), remote.clone()]),
    });
    assert_eq!(h.observe(|v| v.volumes.items[0].size), measurement.size);
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::GatherVolumeSizes { .. }))
    );
    h.checkpoint("volumes-sizes-retained-after-reconnect");
    h.deliver(Event::Connected(Connection {
        name: "other-fixture".into(),
        endpoint: "unix:///other-fixture.sock".into(),
    }));
    assert!(h.observe(|v| v.volumes.items.iter().all(|volume| volume.size.is_none())));
    h.deliver(Event::Disconnected("Fixture Docker disconnect".into()));
    h.checkpoint("volumes-disconnected");
    verify_volume_pruning(h, &[unused, used, remote]);
}

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let started = Instant::now();
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Headless fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.deliver(Event::Snapshot(vec![
        container(
            "fixture-probe-running",
            "headless-fixture-probe-1",
            "running",
            Some("headless-fixture"),
        ),
        container(
            "fixture-probe-stopped",
            "headless-fixture-probe-2",
            "exited",
            Some("headless-fixture"),
        ),
    ]));
    verify_volumes_workspace(&mut h);
    let report = json!({
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820,
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "checks": ["Volumes sidebar and container session preservation", "name driver and project keyboard search", "Age column aligned with sortable headers", "Name Driver Age References ascending and descending with painted row order", "manual size gathering loading duplicate guard and numeric sorting with unavailable last", "size cache retained through Ctrl+R automatic inventory and workspace changes", "gather failure preserves measurements and explicit regather updates sorting", "recreated volume identity invalidates old measurement", "same-endpoint reconnect preserves sizes and disconnect rejects stale gathering", "different Docker endpoint clears cached sizes", "Age numeric ordering with unavailable last in both directions", "Ctrl-click multiple selection and clearing final row", "selection preserved through sorting filtering and fresh inventory", "hidden and protected selections included in review", "Cancel Escape and exact-name single/bulk confirmation", "pending duplicate and conflicting container controls blocked", "partial bulk results retain failed selections", "full bulk success clears selection", "stopped reference guard and container navigation", "10000-row virtualization and search", "Light theme", "stale generations and operation IDs rejected", "bulk disconnection invalidates pending state", "anonymous/all-unused prune dialog Cancel Escape and Enter guard", "anonymous and all-unused prune requests ignore selection and filter", "prune duplicate and conflicting container/image actions blocked", "prune success failure zero/unknown reclaim and size-cache reconciliation", "prune survives workspace change and ignores stale/disconnected completion"]
    });
    h.finish(report);
}
