use super::*;

pub(super) fn verify_container_row_widths(h: &mut Harness) {
    for (width, frame) in [
        (820.0, "container-row-actions-narrow"),
        (1440.0, "container-row-actions-wide"),
    ] {
        let mut sized = Harness::with_size(&h.cx, width, 820.0);
        sized.deliver(Event::Connected(Connection {
            name: "Headless fixture".into(),
            endpoint: "fixture://no-docker".into(),
        }));
        sized.deliver(Event::Snapshot(vec![container(
            "fixture-probe-running",
            "headless-fixture-probe-1",
            "running",
            Some("headless-fixture"),
        )]));
        sized.checkpoint(frame);
        sized
            .cx
            .update_window(sized.window.into(), |_, window, _cx| {
                let row = window.find("row-fixture-probe-running").bounds();
                let restart = window.find("row-restart-fixture-probe-running").bounds();
                let primary = window.find("row-primary-fixture-probe-running").bounds();
                let more = window.find("row-more-fixture-probe-running").bounds();
                assert!(restart.left() >= row.left() + px(130.0));
                assert!(restart.right() <= primary.left());
                assert!(primary.right() <= more.left());
                assert!(
                    more.right() <= row.right(),
                    "Row actions must fit the window"
                );
            })
            .unwrap();
        sized.drain_commands();
        sized.click("row-primary-fixture-probe-running");
        assert!(sized.drain_commands().iter().any(|command| matches!(command, Command::Action { id, action: Action::Stop } if id == "fixture-probe-running")));
        sized.checkpoint(&format!("{frame}-pending"));
        h.checkpoints.extend(sized.checkpoints.iter().cloned());
        h.actions.extend(sized.actions.iter().cloned());
    }
}

pub(super) fn verify_container_row_actions(h: &mut Harness) {
    let dock_height = h.dock_height();
    h.drag_dock(400.0);
    let selected = h.observe(|v| v.containers.selected.clone());
    let terminal = h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id());
    let generation = h.observe(|v| v.terminal.generation);
    let logs_generation = h.observe(|v| v.container_logs.generation);
    let running_only = h.observe(|v| v.containers.running_only);
    let filter = h.observe(|v| v.containers.filter.clone());
    h.click("search");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    if running_only {
        h.click("all");
    }
    h.checkpoint("container-row-actions");
    h.drain_commands();
    h.click("row-restart-fixture-standalone");
    let commands = h.drain_commands();
    assert!(
        matches!(commands.as_slice(), [Command::Action { id, action: Action::Restart }] if id == "fixture-standalone")
    );
    h.checkpoint("container-row-restart-pending");
    h.click("row-restart-fixture-standalone");
    h.click("row-primary-fixture-standalone");
    assert!(
        h.drain_commands().is_empty(),
        "Pending row controls must reject duplicate and conflicting actions"
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-standalone".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();

    h.drain_commands();
    h.click("row-primary-fixture-standalone");
    assert!(
        matches!(h.drain_commands().as_slice(), [Command::Action { id, action: Action::Stop }] if id == "fixture-standalone")
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-standalone".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();

    h.drain_commands();
    h.click("row-primary-fixture-probe-stopped");
    assert!(
        matches!(h.drain_commands().as_slice(), [Command::Action { id, action: Action::Start }] if id == "fixture-probe-stopped")
    );
    h.checkpoint("container-row-start-pending");
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-stopped".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();

    h.click("row-more-fixture-standalone");
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        selected,
        "Opening a row menu must preserve selection"
    );
    h.checkpoint("container-row-menu");
    h.drain_commands();
    h.click_popup_item(8);
    assert!(
        matches!(h.drain_commands().as_slice(), [Command::Action { id, action: Action::Stop }] if id == "fixture-standalone")
    );
    assert_eq!(h.observe(|v| v.containers.selected.clone()), selected);
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    assert_eq!(h.observe(|v| v.terminal.generation), generation);
    assert_eq!(h.observe(|v| v.container_logs.generation), logs_generation);
    assert_eq!(
        h.observe(|v| v.terminal.view.as_ref().unwrap().entity_id()),
        terminal
    );
    assert!(
        h.cx.read_entity(&h.view, |v, cx| v
            .terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .ready),
        "Another container's action must keep the current shell ready"
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-standalone".into(),
        result: Ok(()),
    });
    h.dismiss_image_feedback();
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("container-row-actions-light");
    h.click("settings");
    h.click("theme-ferrydock");
    h.click("settings");
    if running_only {
        h.click("all");
    }
    h.click("search");
    h.key("ctrl-a");
    h.type_text(&filter);
    h.key("escape");
    h.drag_dock(h.dock_height() - dock_height);
    h.drain_commands();
}
