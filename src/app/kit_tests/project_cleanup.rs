use super::*;

pub(super) fn verify_project_cleanup(h: &mut Harness) {
    fn open(h: &mut Harness) {
        h.drain_commands();
        h.click("project-more");
        h.click_popup_item(8);
        assert!(h.observe(|v| v.projects.cleanup.is_some()));
        assert!(
            !h.drain_commands()
                .iter()
                .any(|command| matches!(command, Command::Compose { .. }))
        );
    }
    let projects = h.observe(|v| v.projects.items.clone());
    let containers = h.observe(|v| v.containers.items.clone());
    open(h);
    h.checkpoint("compose-cleanup-review");
    h.click("compose-cleanup-cancel");
    assert!(h.observe(|v| v.projects.cleanup.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Compose { .. }))
    );

    open(h);
    h.key("escape");
    assert!(h.observe(|v| v.projects.cleanup.is_none()));

    open(h);
    h.click("compose-cleanup-volumes");
    h.click("compose-cleanup-images");
    h.checkpoint("compose-cleanup-keep-data");
    h.click("compose-cleanup-confirm");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action:
                    compose::Action::Cleanup {
                        volumes: false,
                        images: false,
                    },
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Reviewed cleanup options must reach worker");
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Removing project containers\n".to_vec(),
    });
    h.checkpoint("compose-cleanup-live");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Err("Fixture network still in use".into()),
    });
    assert!(h.observe(|v| v.projects.items[0].config.is_some()));
    h.checkpoint("compose-cleanup-failed");

    open(h);
    let mut changed = projects.clone();
    changed[0]
        .config
        .as_mut()
        .unwrap()
        .profiles
        .push("changed".into());
    h.deliver(Event::ComposeProjects(changed));
    assert!(h.observe(|v| v.projects.cleanup.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Compose { .. }))
    );
    h.deliver(Event::ComposeProjects(projects.clone()));

    open(h);
    let mut changed = containers.clone();
    changed.push(container(
        "cleanup-new-member",
        "headless-fixture-new",
        "running",
        Some("headless-fixture"),
    ));
    h.deliver(Event::Snapshot(changed));
    assert!(h.observe(|v| v.projects.cleanup.is_none()));
    h.deliver(Event::Snapshot(containers.clone()));

    open(h);
    h.click("compose-cleanup-confirm");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action:
                    compose::Action::Cleanup {
                        volumes: true,
                        images: true,
                    },
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Default cleanup must remove project volumes and local images");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    h.deliver(Event::ComposeProjects(vec![]));
    h.deliver(Event::Snapshot(
        containers
            .iter()
            .filter(|container| container.project.as_deref() != Some("headless-fixture"))
            .cloned()
            .collect(),
    ));
    h.checkpoint("compose-cleanup-completed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-headless-fixture")
            .unwrap()
            .is_none()
    );
    h.deliver(Event::ComposeProjects(projects));
    h.deliver(Event::Snapshot(containers));
}
