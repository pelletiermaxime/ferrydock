// Input-driven favorites flow, included alongside the shared headless harness.
#[test]
fn headless_gpu_favorites_flow() {
    let mut h = Harness::new();
    h.deliver(Event::Connected(Connection {
        name: "Favorites fixture".into(),
        endpoint: "fixture://favorites".into(),
    }));
    let mut instances = vec![
        container(
            "favorite-one",
            "favorite-demo-probe-1",
            "running",
            Some("favorite-demo"),
        ),
        container(
            "favorite-two",
            "favorite-demo-probe-2",
            "running",
            Some("favorite-demo"),
        ),
    ];
    let mut worker = container(
        "favorite-worker",
        "favorite-demo-worker-1",
        "running",
        Some("favorite-demo"),
    );
    worker.service = Some("worker".into());
    instances.push(worker);
    h.deliver(Event::Snapshot(instances.clone()));
    h.settle_ui(200);
    h.click("project-favorite-demo");
    h.click("project-favorites");
    h.capture("favorites-empty");
    h.click("favorite-add");
    h.type_text("Show environment");
    h.click("favorite-command");
    h.type_text("printf 'FAVORITE_READY\\n'; read answer; printf '%s\\n' \"$answer\"");
    h.capture("favorite-editor");
    h.drain_commands();
    h.click("favorite-save");
    h.settle_ui(200);
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::FavoriteTerminal { .. })),
        "Saving cannot run anything"
    );
    let path = h.evidence.join("favorites.json");
    let saved = crate::favorites::Store::load(&path).unwrap();
    assert_eq!(saved.favorites.len(), 1);
    assert_eq!(saved.favorites[0].label, "Show environment");
    assert!(saved.favorites[0].command.contains("read answer"));
    let favorite_id = saved.favorites[0].id;
    h.capture("favorites-saved");
    h.click("favorite-service");
    h.type_text("worker");
    h.key("enter");
    h.click("favorite-add");
    h.type_text("Worker command");
    h.click("favorite-command");
    h.type_text("echo worker");
    h.click("favorite-save");
    h.settle_ui(200);
    let worker_id = crate::favorites::Store::load(&path).unwrap().favorites[1].id;
    assert!(
        h.cx.debug_bounds(h.window.into(), &format!("favorite-review-{worker_id}"))
            .unwrap()
            .is_some(),
        "Service filter must map the correct recipe indices"
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), &format!("favorite-review-{favorite_id}"))
            .unwrap()
            .is_none()
    );
    h.capture("favorites-worker-service");
    h.click("favorite-service");
    h.type_text("probe");
    h.key("enter");
    h.click(&format!("favorite-review-{favorite_id}"));
    h.capture("favorite-after-review");
    h.key("enter");
    h.click("favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::FavoriteTerminal { .. })),
        "Replicas require a deliberate instance selection"
    );
    h.capture("favorite-replica-choice");
    h.click("favorite-instance");
    h.key("down");
    h.key("down");
    h.key("enter");
    h.capture("favorite-ready-to-run");
    h.click("favorite-run");
    let outgoing = h.drain_commands();
    let (id, generation, command) = outgoing
        .into_iter()
        .find_map(|c| match c {
            Command::FavoriteTerminal {
                id,
                generation,
                endpoint,
                target,
                command,
                ..
            } => {
                assert_eq!(endpoint, "fixture://favorites");
                assert_eq!(
                    target,
                    crate::favorites::Target::Service {
                        project: "favorite-demo".into(),
                        service: "probe".into()
                    }
                );
                Some((id, generation, command))
            }
            _ => None,
        })
        .expect("Run must request a favorite terminal");
    assert!(instances.iter().any(|c| c.id == id));
    assert_eq!(command, saved.favorites[0].command);
    assert!(h.observe(|v| v.tab == Tab::Terminal && v.containers.selected.as_ref() == Some(&id)));
    h.settle_ui(150);
    h.deliver(Event::TerminalReady { generation });
    h.deliver(Event::TerminalData {
        generation,
        data: b"FAVORITE_READY\r\n".to_vec(),
    });
    h.type_text("interactive-input");
    h.key("enter");
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::Input { generation: current, data } if *current == generation && data.contains(&b'i'))));
    h.cx.read_entity(&h.view, |v, cx| {
        assert!(
            v.terminal
                .view
                .as_ref()
                .unwrap()
                .read(cx)
                .core
                .visible_text()
                .contains("FAVORITE_READY")
        )
    });
    h.capture("favorite-interactive-terminal");
    h.drain_commands();
    h.click("0-3");
    assert!(h.observe(|view| view.networks_open && view.terminal.view.is_some() && view.terminal.generation == generation));
    assert!(!h.drain_commands().iter().any(|command| matches!(command, Command::CloseTerminal)));
    h.capture("favorite-session-in-networks");
    h.click("0-0");
    assert!(h.observe(|view| !view.networks_open && view.tab == Tab::Terminal && view.terminal.generation == generation));
    h.type_text("after-networks");
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Input { generation: active, data } if *active == generation && data.contains(&b'a'))), "Returning from Networks restores the favorite terminal focus");
    // A live or connecting terminal cannot be replaced by a favorite.
    h.click("container-favorites");
    h.click(&format!("favorite-review-{favorite_id}"));
    h.click("favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::FavoriteTerminal { .. } | Command::CloseTerminal))
    );
    assert_eq!(h.observe(|v| v.terminal.generation), generation);
    h.capture("favorite-terminal-protected");
    h.key("escape");
    h.settle_ui(250);
    h.type_text("still-focused");
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::Input { generation: current, data } if *current == generation && data.contains(&b's'))), "Closing favorites restores terminal focus");
    h.click("disconnect-terminal");
    h.click("container-favorites");
    h.click(&format!("favorite-edit-{favorite_id}"));
    h.key("ctrl-a");
    h.type_text("Updated command");
    h.click("favorite-command");
    h.key("ctrl-a");
    h.type_text("echo updated");
    h.click("favorite-save");
    h.settle_ui(200);
    let updated = crate::favorites::Store::load(&path).unwrap();
    assert_eq!(updated.favorites[0].label, "Updated command");
    assert_eq!(updated.favorites[0].command, "echo updated");
    h.click(&format!("favorite-review-{favorite_id}"));
    let stopped = instances
        .iter()
        .cloned()
        .map(|mut c| {
            c.state = "exited".into();
            c
        })
        .collect::<Vec<_>>();
    h.deliver(Event::Snapshot(stopped.clone()));
    h.click("favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::FavoriteTerminal { .. }))
    );
    h.key("escape");
    h.settle_ui(250);
    h.click("all");
    h.capture("favorites-stopped-list");
    h.click("project-favorite-demo");
    h.click("project-favorites");
    h.click(&format!("favorite-review-{favorite_id}"));
    h.capture("favorite-service-stopped");
    h.click("favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::FavoriteTerminal { .. }))
    );
    h.deliver(Event::Snapshot(vec![]));
    h.click("favorite-back");
    h.click(&format!("favorite-review-{favorite_id}"));
    h.capture("favorite-service-absent");
    h.deliver(Event::Connected(Connection {
        name: "Other fixture".into(),
        endpoint: "fixture://other".into(),
    }));
    h.click("favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::FavoriteTerminal { .. }))
    );
    h.capture("favorite-endpoint-changed");
    h.key("escape");
    h.settle_ui(250);
    h.deliver(Event::Connected(Connection {
        name: "Favorites fixture".into(),
        endpoint: "fixture://favorites".into(),
    }));
    let recreated = container(
        "favorite-recreated",
        "favorite-demo-probe-1",
        "running",
        Some("favorite-demo"),
    );
    h.deliver(Event::Snapshot(vec![recreated]));
    h.click("project-favorites");
    h.click(&format!("favorite-review-{favorite_id}"));
    h.drain_commands();
    h.click("favorite-run");
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::FavoriteTerminal { id, command, .. } if id == "favorite-recreated" && command == "echo updated")), "Saved service recipe follows a recreated container");
    h.capture("favorite-recreated-target");
    h.click("disconnect-terminal");
    h.click("container-favorites");
    h.click(&format!("favorite-delete-{favorite_id}"));
    h.settle_ui(200);
    let remaining = crate::favorites::Store::load(&path).unwrap();
    assert_eq!(remaining.favorites.len(), 1);
    assert_eq!(remaining.favorites[0].id, worker_id);
    h.click("favorite-service");
    h.type_text("worker");
    h.key("enter");
    h.click(&format!("favorite-delete-{worker_id}"));
    h.settle_ui(200);
    assert!(
        crate::favorites::Store::load(&path)
            .unwrap()
            .favorites
            .is_empty()
    );
    h.capture("favorite-deleted");
    h.key("escape");
    h.settle_ui(250);
    std::fs::write(h.evidence.join("headless.json"), serde_json::to_vec_pretty(&json!({
        "status":"passed", "screenshots":h.screenshots, "frame_timings":h.frame_timings,
        "checks":["project and container entry points", "create edit delete and local persistence", "manual run only", "explicit replica selection", "exact command and endpoint request", "interactive input and visible output", "existing terminal and focus preserved", "Networks preserves favorite terminal and restores focus", "stopped absent stale-instance and changed-endpoint guards"]
    })).unwrap()).unwrap();
}
