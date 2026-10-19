use super::*;

fn select(h: &mut Harness, n: usize) {
    h.click("search");
    h.key("ctrl-a");
    h.type_text(&format!("terminal-probe-{n}"));
    h.key("escape");
    h.click(&format!("row-terminal-{n}"));
}

fn open(h: &mut Harness, n: usize) -> u64 {
    select(h, n);
    h.click_tab("container-tabs", 1);
    h.drain_commands();
    h.click("open-shell");
    h.drain_commands()
        .into_iter()
        .find_map(|c| match c {
            Command::Terminal { generation, .. } => Some(generation),
            _ => None,
        })
        .expect("Open shell must request a session")
}

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Terminal fixture".into(),
        endpoint: "fixture://terminal".into(),
    }));
    let mut containers = (0..7)
        .map(|n| {
            container(
                &format!("terminal-{n}"),
                &format!("terminal-probe-{n}"),
                "running",
                None,
            )
        })
        .collect::<Vec<_>>();
    h.deliver(Event::Snapshot(containers.clone()));
    h.settle_ui(150);
    let mut generations = vec![];
    for n in 0..6 {
        let generation = open(&mut h, n);
        generations.push(generation);
        // Navigation while connecting must retain the request too.
        if n > 0 {
            h.deliver(Event::TerminalReady {
                generation: generations[n - 1],
            });
            h.deliver(Event::TerminalData {
                generation: generations[n - 1],
                data: format!("HIDDEN_SESSION_{n}\r\n").into_bytes(),
            });
        }
    }
    h.deliver(Event::TerminalReady {
        generation: generations[5],
    });
    select(&mut h, 6);
    h.click_tab("container-tabs", 1);
    h.drain_commands();
    h.click("open-shell");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::Terminal { .. } | Command::CloseTerminal { .. }))
    );
    assert!(h.observe(|v| v.terminal.view.is_none()));
    h.checkpoint("terminal-session-limit-keeps-six-shells");
    h.dismiss_image_feedback();

    select(&mut h, 0);
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    h.cx.read_entity(&h.view, |v, cx| {
        let terminal = v.terminal.view.as_ref().unwrap().read(cx);
        assert!(terminal.ready);
        assert!(terminal.core.visible_text().contains("HIDDEN_SESSION_1"));
    });
    // A stopped unselected container loses only its own attachment. Late ready
    // and data events cannot revive it or contaminate the displayed terminal.
    containers[1].state = "exited".into();
    h.drain_commands();
    h.deliver(Event::Snapshot(containers.clone()));
    let commands = h.drain_commands();
    assert!(commands.iter().any(
        |c| matches!(c, Command::CloseTerminal { generation } if *generation == generations[1])
    ));
    assert!(!commands.iter().any(
        |c| matches!(c, Command::CloseTerminal { generation } if *generation == generations[0])
    ));
    h.deliver(Event::TerminalReady {
        generation: generations[1],
    });
    h.deliver(Event::TerminalData {
        generation: generations[1],
        data: b"STALE_STOPPED_OUTPUT\r\n".to_vec(),
    });
    assert!(h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).ready
    }));
    h.checkpoint("terminal-hidden-container-stop-is-scoped");

    // An ended view can release a slot, without evicting a live shell.
    let seventh = open(&mut h, 6);
    assert!(seventh > *generations.iter().max().unwrap());
    h.deliver(Event::TerminalReady {
        generation: seventh,
    });
    h.click("disconnect-terminal");
    assert!(
        h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::CloseTerminal { generation } if *generation == seventh))
    );
    let reopened = open(&mut h, 6);
    assert!(reopened > seventh);
    h.deliver(Event::TerminalReady {
        generation: seventh,
    });
    assert!(!h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).ready
    }));
    h.deliver(Event::TerminalReady {
        generation: reopened,
    });
    h.checkpoint("terminal-reopen-rejects-old-session-events");

    // A hidden shell can exit naturally; returning keeps its final output/status.
    h.deliver(Event::TerminalData {
        generation: generations[0],
        data: b"FINAL_HIDDEN_OUTPUT\r\n".to_vec(),
    });
    h.deliver(Event::TerminalEnded {
        generation: generations[0],
        error: None,
    });
    select(&mut h, 0);
    h.cx.read_entity(&h.view, |v, cx| {
        let terminal = v.terminal.view.as_ref().unwrap().read(cx);
        assert!(!terminal.ready);
        assert!(terminal.status.contains("Shell exited"));
        assert!(terminal.core.visible_text().contains("FINAL_HIDDEN_OUTPUT"));
    });
    h.checkpoint("terminal-hidden-exit-retains-output");

    // Removing a hidden container forgets its view and never retargets its PTY.
    containers.retain(|c| c.id != "terminal-2");
    h.drain_commands();
    h.deliver(Event::Snapshot(containers.clone()));
    assert!(h.drain_commands().iter().any(
        |c| matches!(c, Command::CloseTerminal { generation } if *generation == generations[2])
    ));
    h.deliver(Event::Disconnected("Fixture disconnected".into()));
    h.deliver(Event::TerminalReady {
        generation: generations[3],
    });
    h.deliver(Event::Connected(Connection {
        name: "Terminal fixture".into(),
        endpoint: "fixture://terminal".into(),
    }));
    select(&mut h, 3);
    assert!(!h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).ready
    }));
    assert!(h.cx.read_entity(&h.view, |v, cx| {
        v.terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .status
            .contains("Docker disconnected")
    }));
    h.checkpoint("terminal-disconnect-invalidates-hidden-sessions");
    h.finish(json!({ "checks": ["six bounded per-container shells", "pending shell navigation", "background output routing", "scoped hidden stop/removal", "slot recovery", "monotonic session identities", "stale event rejection", "hidden natural exit", "disconnect invalidation"] }));
}
