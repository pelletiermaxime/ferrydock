use super::*;

// User-facing quick launcher, pins, replica selection, and independent consoles.
#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let mut h = Harness::new(test_cx);
    let endpoint = "fixture://quick-run";
    h.deliver(Event::Connected(Connection {
        name: "Quick-run fixture".into(),
        endpoint: endpoint.into(),
    }));
    let mut worker = container(
        "quick-worker",
        "quick-demo-laravel-1",
        "running",
        Some("quick-demo"),
    );
    worker.service = Some("laravel".into());
    let one = container(
        "quick-one",
        "quick-demo-probe-1",
        "running",
        Some("quick-demo"),
    );
    let two = container(
        "quick-two",
        "quick-demo-probe-2",
        "running",
        Some("quick-demo"),
    );
    let mut instances = vec![worker, one, two];
    h.deliver(Event::Snapshot(instances.clone()));
    h.settle_ui(150);
    h.click("project-quick-demo");
    h.click("project-favorites");
    h.click("favorite-add");
    h.type_text("Composer install");
    h.click("favorite-command");
    h.type_text("composer install");
    h.click("favorite-save");
    h.settle_ui(150);
    let path = h.evidence.join("favorites.json");
    let composer = crate::favorites::Store::load(&path).unwrap().favorites[0].id;
    h.click(&format!("favorite-pin-{composer}"));
    h.settle_ui(150);
    assert!(crate::favorites::Store::load(&path).unwrap().favorites[0].pinned);
    h.checkpoint("quick-pin-manager");
    h.click("favorite-service");
    h.type_text("probe");
    h.key("enter");
    h.click("favorite-add");
    h.type_text("Probe environment");
    h.click("favorite-command");
    h.type_text("printf 'probe\\n'");
    h.click("favorite-save");
    h.settle_ui(150);
    h.key("escape");
    h.settle_ui(150);
    h.checkpoint("quick-project-pin");
    let config = ComposeConfig {
        project: "quick-demo".into(),
        directory: "/fixture/quick-demo".into(),
        files: vec!["compose.yaml".into()],
        ..Default::default()
    };
    h.deliver(Event::ComposeProjects(vec![ComposeProject {
        name: "quick-demo".into(),
        config: Some(config.clone()),
        hint: config,
        worktree: None,
    }]));
    h.drain_commands();
    h.click("compose-restart");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::Compose {
                operation,
                action: compose::Action::Restart,
                ..
            } => Some(operation),
            _ => None,
        })
        .expect("Project restart request");
    h.key("ctrl-k");
    h.settle_ui(150);
    h.type_text("composer");
    h.settle_ui(150);
    h.key("enter");
    h.settle_ui(150);
    assert!(!h.drain_commands().iter().any(|c| matches!(
        c,
        Command::QuickTerminal { .. } | Command::RunFavorite { .. }
    )));
    h.checkpoint("quick-project-restart-blocked");
    h.key("escape");
    h.settle_ui(150);
    h.deliver(Event::ComposeFinished {
        project: "quick-demo".into(),
        operation,
        result: Ok(()),
    });
    h.deliver(Event::ActionStarted {
        id: "quick-worker".into(),
        action: Action::Restart,
    });
    h.drain_commands();
    h.click(&format!("quick-pin-{composer}"));
    h.settle_ui(150);
    assert!(!h.drain_commands().iter().any(|c| matches!(
        c,
        Command::QuickTerminal { .. } | Command::RunFavorite { .. }
    )));
    h.checkpoint("quick-target-restart-blocked");
    h.key("escape");
    h.settle_ui(150);
    h.deliver(Event::ActionFinished {
        id: "quick-worker".into(),
        result: Ok(()),
    });
    // A restart on another service in the same project must not block this target,
    // including the readiness check before the command is sent to its new shell.
    h.deliver(Event::ActionStarted {
        id: "quick-one".into(),
        action: Action::Restart,
    });
    h.drain_commands();
    h.key("ctrl-k");
    h.settle_ui(150);
    h.type_text("composer");
    h.settle_ui(150);
    h.checkpoint("quick-launcher-search");
    assert!(!h.drain_commands().iter().any(|c| matches!(
        c,
        Command::QuickTerminal { .. } | Command::RunFavorite { .. }
    )));
    h.key("enter");
    h.settle_ui(150);
    let first = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::QuickTerminal { id, generation } => {
                assert_eq!(id, "quick-worker");
                Some(generation)
            }
            _ => None,
        })
        .expect("Enter opens target shell");
    h.checkpoint("quick-connecting");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::RunFavorite { .. }))
    );
    h.deliver(Event::TerminalReady { generation: first });
    h.deliver(Event::TerminalPrompt {
        generation: first,
        input_count: 0,
        at_prompt: true,
    });
    h.deliver(Event::TerminalData {
        generation: first,
        data: b"www-data@laravel:/app$ ".to_vec(),
    });
    h.settle_ui(150);
    let run = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::RunFavorite {
                id,
                generation,
                run,
                command,
                ..
            } => {
                assert_eq!(id, "quick-worker");
                assert_eq!(generation, first);
                assert_eq!(command, "composer install");
                Some(run)
            }
            _ => None,
        })
        .expect("Only ready shell receives command");
    assert!(h.observe(|v| v.containers.busy.contains_key("quick-one")));
    h.checkpoint("quick-unrelated-restart-allowed");
    h.deliver(Event::ActionFinished {
        id: "quick-one".into(),
        result: Ok(()),
    });
    h.deliver(Event::FavoriteStarted {
        generation: first,
        run,
    });
    h.deliver(Event::TerminalData {
        generation: first,
        data: b"composer install\r\nInstalling dependencies...\r\n".to_vec(),
    });
    h.checkpoint("quick-running");
    h.click("quick-hide");
    h.settle_ui(150);
    // An ordinary interactive shell stays alive while the launcher operates.
    h.click("row-quick-one");
    h.key("enter");
    let regular = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::Terminal { generation, .. } => Some(generation),
            _ => None,
        })
        .unwrap();
    h.deliver(Event::TerminalReady {
        generation: regular,
    });
    h.deliver(Event::TerminalPrompt {
        generation: regular,
        input_count: 0,
        at_prompt: true,
    });
    h.type_text("unfinished");
    h.drain_commands();
    h.key("ctrl-k");
    h.settle_ui(150);
    h.type_text("composer");
    h.settle_ui(150);
    h.key("enter");
    h.settle_ui(150);
    let cmds = h.drain_commands();
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::CloseTerminal | Command::Input { .. } | Command::Terminal { .. }
    )));
    let second = cmds
        .into_iter()
        .find_map(|c| match c {
            Command::QuickTerminal { generation, .. } => Some(generation),
            _ => None,
        })
        .expect("Busy quick shell preserved, new shell opened");
    assert_ne!(first, second);
    h.checkpoint("quick-busy-session-preserved");
    h.deliver(Event::TerminalReady { generation: second });
    h.deliver(Event::TerminalPrompt {
        generation: second,
        input_count: 0,
        at_prompt: true,
    });
    h.settle_ui(150);
    let run2 = h
        .drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::RunFavorite {
                generation, run, ..
            } if generation == second => Some(run),
            _ => None,
        })
        .unwrap();
    h.deliver(Event::FavoriteStarted {
        generation: second,
        run: run2,
    });
    h.deliver(Event::FavoriteFinished {
        generation: second,
        run: run2,
        result: Ok(1),
    });
    h.deliver(Event::TerminalPrompt {
        generation: second,
        input_count: 0,
        at_prompt: true,
    });
    h.checkpoint("quick-failed");
    h.deliver(Event::FavoriteFinished {
        generation: first,
        run,
        result: Ok(0),
    });
    h.deliver(Event::TerminalPrompt {
        generation: first,
        input_count: 0,
        at_prompt: true,
    });
    h.deliver(Event::TerminalData {
        generation: first,
        data: b"Generating autoload files... done\r\nwww-data@laravel:/app$ ".to_vec(),
    });
    h.click(&format!("quick-session-{first}"));
    h.checkpoint("quick-completed");
    // Reuse the idle quick shell; regular partial input is not touched.
    h.click("quick-another");
    h.settle_ui(150);
    h.type_text("composer");
    h.settle_ui(150);
    h.key("enter");
    h.settle_ui(150);
    let cmds = h.drain_commands();
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::QuickTerminal { .. } | Command::CloseTerminal | Command::Input { .. }
    )));
    assert!(
        cmds.iter()
            .any(|c| matches!(c,Command::RunFavorite{generation,..}if *generation==first))
    );
    h.click("quick-hide");
    h.settle_ui(150);
    let saved = crate::favorites::Store::load(&path).unwrap();
    assert!(
        saved
            .favorites
            .iter()
            .find(|f| f.id == composer)
            .unwrap()
            .use_count
            >= 2
    );
    // Pin opens the same quick path directly from a project header.
    h.drain_commands();
    h.click(&format!("quick-pin-{composer}"));
    h.settle_ui(150);
    assert!(
        h.cx.debug_bounds(h.window.into(), "quick-run-terminal")
            .unwrap()
            .is_some()
    );
    h.click("quick-hide");
    h.settle_ui(150);
    h.key("ctrl-k");
    h.settle_ui(150);
    h.type_text("probe");
    h.settle_ui(150);
    h.key("enter");
    h.settle_ui(150);
    h.checkpoint("quick-replica-picker");
    h.drain_commands();
    h.key("down");
    h.key("enter");
    h.settle_ui(150);
    let cmds = h.drain_commands();
    let replica = cmds
        .into_iter()
        .find_map(|c| match c {
            Command::QuickTerminal { id, generation } => {
                assert_eq!(id, "quick-two");
                Some(generation)
            }
            _ => None,
        })
        .unwrap();
    // Recreated/stopped target while connecting must never receive the command.
    instances.retain(|c| c.id != "quick-two");
    h.deliver(Event::Snapshot(instances.clone()));
    h.settle_ui(150);
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c,Command::RunFavorite{generation,..}if *generation==replica))
    );
    h.checkpoint("quick-target-disappeared");
    h.click("quick-hide");
    h.settle_ui(150);
    for c in &mut instances {
        if c.id == "quick-worker" {
            c.state = "exited".into();
        }
    }
    h.deliver(Event::Snapshot(instances));
    h.settle_ui(150);
    h.key("ctrl-k");
    h.settle_ui(150);
    h.type_text("composer");
    h.settle_ui(150);
    h.drain_commands();
    h.key("enter");
    h.settle_ui(150);
    assert!(!h.drain_commands().iter().any(|c| matches!(
        c,
        Command::QuickTerminal { .. } | Command::RunFavorite { .. }
    )));
    h.checkpoint("quick-target-stopped");
    h.key("escape");
    h.settle_ui(150);
    h.deliver(Event::Disconnected("fixture disconnect".into()));
    h.deliver(Event::Connected(Connection {
        name: "Other endpoint".into(),
        endpoint: "fixture://other".into(),
    }));
    h.key("ctrl-k");
    h.settle_ui(150);
    h.checkpoint("quick-other-endpoint-empty");
    h.key("enter");
    assert!(!h.drain_commands().iter().any(|c| matches!(
        c,
        Command::QuickTerminal { .. } | Command::RunFavorite { .. }
    )));
    h.key("escape");
    h.settle_ui(150);
    h.finish(json!({"checks":["Ctrl+K from workspace and terminal", "search then Enter runs correct target", "unrelated same-project restart permits launch and pending command", "target and Compose project restarts block launcher and pin", "no command before shell prompt", "busy regular and quick shell preservation", "idle quick shell reuse", "completed/failed results", "pin persistence and project-header launch", "recent usage persistence", "explicit replica choice", "stopped/disappeared target and endpoint guards"]}));
}
