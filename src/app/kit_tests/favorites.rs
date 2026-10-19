use super::*;

fn select_replica(h: &mut Harness, n: usize) {
    h.click("search");
    h.key("ctrl-a");
    h.type_text(&format!("favorite-demo-probe-{n}"));
    h.key("escape");
    h.click(if n == 1 {
        "row-favorite-one"
    } else {
        "row-favorite-two"
    });
}

// Input-driven favorites flow using the real terminal toolbar and a fixture worker.
#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::terminal::FavoriteStatus;
    let mut h = Harness::new(test_cx);
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
    h.settle_ui(150);
    h.click("project-favorite-demo");
    h.click("project-favorites");
    h.checkpoint("favorites-manager-empty");
    h.click("favorite-add");
    h.type_text("Show environment");
    h.click("favorite-command");
    h.type_text("printf 'READY\\n'; read answer; printf '%s\\n' \"$answer\"");
    h.checkpoint("favorites-editor");
    h.drain_commands();
    h.click("favorite-save");
    h.settle_ui(150);
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. } | Command::Terminal { .. }))
    );
    let path = h.evidence.join("favorites.json");
    let saved = crate::favorites::Store::load(&path).unwrap();
    let favorite_id = saved.favorites[0].id;
    h.checkpoint("favorites-manager-saved");
    h.click("favorite-service");
    h.type_text("worker");
    h.key("enter");
    h.click("favorite-add");
    h.type_text("Worker task");
    h.click("favorite-command");
    h.type_text("echo worker");
    h.click("favorite-save");
    h.settle_ui(150);
    let worker_id = crate::favorites::Store::load(&path).unwrap().favorites[1].id;
    assert!(
        h.cx.debug_bounds(h.window.into(), &format!("favorite-edit-{worker_id}"))
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), &format!("favorite-edit-{favorite_id}"))
            .unwrap()
            .is_none()
    );
    h.key("escape");
    h.settle_ui(150);
    h.click("row-favorite-one");
    h.click_tab("container-tabs", 1);
    h.checkpoint("favorites-terminal-open-shell-first");
    h.drain_commands();
    h.click("terminal-favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. } | Command::Terminal { .. }))
    );
    h.click("open-shell");
    let generation = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::Terminal { id, generation, .. } => {
                assert_eq!(id, "favorite-one");
                Some(generation)
            }
            _ => None,
        })
        .unwrap();
    h.settle_ui(150);
    h.deliver(Event::TerminalReady { generation });
    h.deliver(Event::TerminalPrompt {
        generation,
        input_count: 0,
        at_prompt: true,
    });
    h.deliver(Event::TerminalData {
        generation,
        data: b"/app # ".to_vec(),
    });
    h.click("terminal-favorite-picker");
    h.type_text("Show environment");
    h.key("enter");
    h.checkpoint("favorites-terminal-ready");
    h.deliver(Event::ActionStarted {
        id: "favorite-one".into(),
        action: Action::Restart,
    });
    h.drain_commands();
    h.click("terminal-favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. }))
    );
    h.checkpoint("favorites-target-restart-blocked");
    h.deliver(Event::ActionFinished {
        id: "favorite-one".into(),
        result: Ok(()),
    });
    // A different replica of this service can restart while this shell stays ready.
    h.deliver(Event::ActionStarted {
        id: "favorite-two".into(),
        action: Action::Restart,
    });
    h.drain_commands();
    h.click("terminal-favorite-run");
    let run = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::RunFavorite {
                id,
                generation: current,
                run,
                endpoint,
                target,
                command,
            } => {
                assert_eq!(id, "favorite-one");
                assert_eq!(current, generation);
                assert_eq!(endpoint, "fixture://favorites");
                assert_eq!(
                    target,
                    crate::favorites::Target::Service {
                        project: "favorite-demo".into(),
                        service: "probe".into()
                    }
                );
                assert_eq!(command, saved.favorites[0].command);
                Some(run)
            }
            Command::Terminal { .. } | Command::CloseTerminal { .. } => {
                panic!("Running a favorite must preserve the current shell")
            }
            _ => None,
        })
        .unwrap();
    assert!(h.observe(|v| v.containers.busy.contains_key("favorite-two")));
    h.checkpoint("favorites-unrelated-restart-allowed");
    h.deliver(Event::ActionFinished {
        id: "favorite-two".into(),
        result: Ok(()),
    });
    h.cx.read_entity(&h.view, |v, cx| {
        assert!(matches!(
            v.terminal
                .view
                .as_ref()
                .unwrap()
                .read(cx)
                .favorite
                .as_ref()
                .unwrap()
                .status,
            FavoriteStatus::Starting
        ))
    });
    h.settle_ui(150);
    h.deliver(Event::FavoriteStarted { generation, run });
    h.cx.read_entity(&h.view, |v, cx| {
        assert!(matches!(
            v.terminal
                .view
                .as_ref()
                .unwrap()
                .read(cx)
                .favorite
                .as_ref()
                .unwrap()
                .status,
            FavoriteStatus::Running
        ))
    });
    h.deliver(Event::TerminalData {
        generation,
        data: b"printf 'READY\\n'; read answer; printf '%s\\n' \"$answer\"\r\nREADY\r\n".to_vec(),
    });
    h.type_text("interactive-proof");
    h.key("enter");
    assert!(h.drain_commands().iter().any(
        |c| matches!(c, Command::Input { generation: current, .. } if *current == generation)
    ));
    h.checkpoint("favorites-terminal-running");
    // A busy favorite survives navigation and another container's regular shell.
    let original_view = h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id());
    h.drain_commands();
    select_replica(&mut h, 2);
    assert!(h.observe(|v| v.terminal.view.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::CloseTerminal { .. } | Command::Terminal { .. }))
    );
    h.click_tab("container-tabs", 1);
    h.click("open-shell");
    let other_generation = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::Terminal { generation, id, .. } => {
                assert_eq!(id, "favorite-two");
                Some(generation)
            }
            _ => None,
        })
        .unwrap();
    assert_ne!(generation, other_generation);
    h.deliver(Event::TerminalReady {
        generation: other_generation,
    });
    h.deliver(Event::TerminalData {
        generation,
        data: b"STILL_WATCHING_WHILE_AWAY\r\n".to_vec(),
    });
    h.deliver(Event::TerminalData {
        generation: other_generation,
        data: b"OTHER_CONTAINER_ONLY\r\n".to_vec(),
    });
    select_replica(&mut h, 1);
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    assert_eq!(h.observe(|v| v.terminal.generation), generation);
    assert_eq!(
        h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id()),
        original_view
    );
    h.cx.read_entity(&h.view, |v, cx| {
        let terminal = v.terminal.view.as_ref().unwrap().read(cx);
        assert!(terminal.ready);
        assert!(
            terminal
                .core
                .visible_text()
                .contains("STILL_WATCHING_WHILE_AWAY")
        );
        assert!(
            !terminal
                .core
                .visible_text()
                .contains("OTHER_CONTAINER_ONLY")
        );
        assert!(matches!(
            terminal.favorite.as_ref().unwrap().status,
            FavoriteStatus::Running
        ));
    });
    // Enter on the container list reveals this same shell instead of replacing it.
    h.key("enter");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::CloseTerminal { .. } | Command::Terminal { .. }))
    );
    h.checkpoint("favorites-running-shell-restored-after-container-switch");

    h.click("terminal-favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. }))
    );
    let input_count = h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).input_count
    });
    select_replica(&mut h, 2);
    h.deliver(Event::FavoriteFinished {
        generation,
        run,
        result: Ok(0),
    });
    h.deliver(Event::TerminalPrompt {
        generation,
        input_count,
        at_prompt: true,
    });
    h.click("disconnect-terminal");
    assert!(h.drain_commands().iter().any(|c| matches!(c,
        Command::CloseTerminal { generation: g } if *g == other_generation)));
    select_replica(&mut h, 1);
    assert_eq!(
        h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id()),
        original_view
    );
    h.checkpoint("favorites-hidden-completion-and-scoped-disconnect");
    h.deliver(Event::TerminalData {
        generation,
        data: b"interactive-proof\r\n/app # ".to_vec(),
    });
    h.cx.read_entity(&h.view, |v, cx| {
        let t = v.terminal.view.as_ref().unwrap().read(cx);
        assert!(t.ready && t.at_prompt);
        assert!(matches!(
            t.favorite.as_ref().unwrap().status,
            FavoriteStatus::Completed(0)
        ));
        assert!(t.core.visible_text().contains("interactive-proof"));
    });
    h.checkpoint("favorites-terminal-completed");
    h.require_visible("terminal-favorite-status");
    // Repeated runs stay in the same shell; failure is visible and keeps the prompt.
    h.click("terminal-favorite-run");
    let second_run = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::RunFavorite {
                run,
                generation: current,
                ..
            } => {
                assert_eq!(current, generation);
                Some(run)
            }
            _ => None,
        })
        .unwrap();
    h.deliver(Event::FavoriteFinished {
        generation,
        run: second_run,
        result: Ok(7),
    });
    h.deliver(Event::TerminalPrompt {
        generation,
        input_count,
        at_prompt: true,
    });
    h.deliver(Event::TerminalData {
        generation,
        data: b"example command failure\r\n/app # ".to_vec(),
    });
    h.checkpoint("favorites-terminal-failed");
    h.require_visible("terminal-favorite-status");
    h.type_text("unfinished-input");
    h.click("terminal-favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. }))
    );
    h.checkpoint("favorites-terminal-input-protected");
    h.key("ctrl-c");
    let input_count = h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).input_count
    });
    h.deliver(Event::TerminalPrompt {
        generation,
        input_count,
        at_prompt: true,
    });
    h.deliver(Event::FavoriteFinished {
        generation,
        run,
        result: Ok(0),
    });
    h.deliver(Event::FavoriteFinished {
        generation: generation + 90,
        run: second_run,
        result: Ok(0),
    });
    h.cx.read_entity(&h.view, |v, cx| {
        assert!(matches!(
            v.terminal
                .view
                .as_ref()
                .unwrap()
                .read(cx)
                .favorite
                .as_ref()
                .unwrap()
                .status,
            FavoriteStatus::Completed(7)
        ))
    });
    // Management preserves the existing terminal, and saving still never executes.
    h.click("terminal-favorite-manage");
    h.click(&format!("favorite-edit-{favorite_id}"));
    h.key("ctrl-a");
    h.type_text("Updated command");
    h.click("favorite-command");
    h.key("ctrl-a");
    h.type_text("echo updated");
    h.click("favorite-save");
    h.settle_ui(150);
    h.key("escape");
    h.settle_ui(150);
    assert_eq!(h.observe(|v| v.terminal.generation), generation);
    assert!(!h.drain_commands().iter().any(|c| matches!(
        c,
        Command::RunFavorite { .. } | Command::CloseTerminal { .. } | Command::Terminal { .. }
    )));
    h.type_text("focus-restored");
    assert!(
        h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::Input { .. }))
    );
    // Scope changes must never send to another endpoint or another replica.
    h.deliver(Event::Connected(Connection {
        name: "Other endpoint".into(),
        endpoint: "fixture://other".into(),
    }));
    h.click("terminal-favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. }))
    );
    h.checkpoint("favorites-other-endpoint");
    h.deliver(Event::Connected(Connection {
        name: "Favorites fixture".into(),
        endpoint: "fixture://favorites".into(),
    }));
    let mut stopped = instances.clone();
    for c in &mut stopped {
        c.state = "exited".into();
    }
    h.deliver(Event::Snapshot(stopped));
    h.click("terminal-favorite-run");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. }))
    );
    h.checkpoint("favorites-stopped-container");
    let recreated = container(
        "favorite-recreated",
        "favorite-demo-probe-1",
        "running",
        Some("favorite-demo"),
    );
    h.deliver(Event::Snapshot(vec![recreated]));
    h.click_tab("container-tabs", 1);
    h.click("open-shell");
    let generation = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::Terminal { id, generation, .. } => {
                assert_eq!(id, "favorite-recreated");
                Some(generation)
            }
            _ => None,
        })
        .unwrap();
    h.deliver(Event::TerminalReady { generation });
    h.deliver(Event::TerminalPrompt {
        generation,
        input_count: 0,
        at_prompt: true,
    });
    h.click("terminal-favorite-picker");
    h.type_text("Updated command");
    h.key("enter");
    h.click("terminal-favorite-run");
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::RunFavorite { id, command, .. } if id == "favorite-recreated" && command == "echo updated")));
    h.checkpoint("favorites-recreated-container");
    h.click("terminal-favorite-manage");
    h.click(&format!("favorite-delete-{favorite_id}"));
    h.settle_ui(150);
    assert_eq!(
        crate::favorites::Store::load(&path)
            .unwrap()
            .favorites
            .len(),
        1
    );
    h.click("favorite-service");
    h.type_text("worker");
    h.key("enter");
    h.click(&format!("favorite-delete-{worker_id}"));
    h.settle_ui(150);
    assert!(
        crate::favorites::Store::load(&path)
            .unwrap()
            .favorites
            .is_empty()
    );
    h.checkpoint("favorites-deleted");
    h.key("escape");
    h.settle_ui(150);
    h.finish(json!({
        "checks":["manage/create/edit/delete and service/endpoint persistence", "Kit favorites selector and Run above terminal", "same-shell requests and explicit current replica", "running/completed/failed exit status", "interactive input and visible output", "busy/partial-input guards", "stale run/session guards", "management focus/session preservation", "stopped/endpoint/recreated guards"]
    }));
}
