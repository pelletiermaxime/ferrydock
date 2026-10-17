use super::*;

pub(super) fn verify_image_prune(h: &mut Harness, inventory: &[crate::images::Image]) {
    use crate::images::PruneMode;
    let selection = h.observe(|v| v.images.selection.clone());
    h.drain_commands();
    h.click("image-prune");
    assert!(h.observe(|v| v.images.prune_review));
    h.checkpoint("images-prune-review");
    h.key("enter");
    assert!(h.observe(|v| v.images.prune_review && v.images.pruning.is_none()));
    h.click("image-prune-cancel");
    assert!(!h.observe(|v| v.images.prune_review));
    h.click("image-prune");
    h.key("escape");
    assert!(!h.observe(|v| v.images.prune_review));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::PruneImages { .. }))
    );

    // Search/selection must not become filters on this daemon-wide action.
    h.key("ctrl-f");
    h.type_text("postgres");
    h.key("escape");
    for (selector, expected, result) in [
        (
            "image-prune-untagged",
            PruneMode::Untagged,
            Ok(Some(2 * 1_048_576)),
        ),
        (
            "image-prune-unused",
            PruneMode::AllUnused,
            Err("fixture prune denied".into()),
        ),
        ("image-prune-unused", PruneMode::AllUnused, Ok(None)),
    ] {
        h.click("image-prune");
        h.click(selector);
        let operation = h.observe(|v| v.images.pruning.unwrap());
        assert!(!h.observe(|v| v.images.prune_review));
        assert_eq!(h.drain_commands().iter().filter(|command| matches!(command, Command::PruneImages { mode, operation: current } if *mode == expected && *current == operation)).count(), 1);
        h.click("image-prune");
        h.click("image-remove");
        assert!(
            h.drain_commands().is_empty(),
            "Pending prune blocks duplicate and removal requests"
        );
        h.checkpoint("images-prune-pending");
        h.deliver(Event::ImagesPruned {
            operation: operation + 1,
            result: Ok(Some(99)),
        });
        assert_eq!(h.observe(|v| v.images.pruning), Some(operation));
        h.deliver(Event::ImagesPruned { operation, result });
        assert!(h.observe(|v| v.images.pruning.is_none()));
        assert!(
            h.drain_commands()
                .iter()
                .any(|command| matches!(command, Command::Images { .. }))
        );
        h.settle_ui(300);
        assert!(
            h.notification_count() > 0,
            "Prune completion must show feedback"
        );
        h.checkpoint(if expected == PruneMode::Untagged {
            "images-prune-success"
        } else {
            "images-prune-feedback"
        });
        h.dismiss_image_feedback();
        let generation = h.observe(|v| v.images.generation);
        h.deliver(Event::Images {
            generation,
            result: Ok(inventory.to_vec()),
        });
        assert_eq!(h.observe(|v| v.images.selection.clone()), selection);
    }
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.drain_commands();
}

pub(super) fn verify_images_workspace(h: &mut Harness) {
    use crate::images::{Image, ImageContainer};
    let connection = h.observe(|v| v.connection.clone().unwrap());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let unused = Image {
        id: format!("sha256:{}", "a".repeat(64)),
        tags: vec!["ferrydock-demo:latest".into()],
        digests: vec![],
        size: Some(128 * 1_048_576),
        created: now - 2 * 86_400,
        containers: vec![].into(),
    };
    let used = Image {
        id: format!("sha256:{}", "b".repeat(64)),
        tags: vec!["postgres:demo".into()],
        digests: vec![],
        size: Some(32 * 1_048_576),
        created: now - 3 * 86_400,
        containers: vec![ImageContainer {
            id: "fixture-probe-stopped".into(),
            name: "demo-database-stopped".into(),
            state: "exited".into(),
        }]
        .into(),
    };
    let untagged = Image {
        id: format!("sha256:{}", "c".repeat(64)),
        tags: vec![],
        size: None,
        created: 0,
        containers: vec![].into(),
        ..unused.clone()
    };
    let inventory = vec![unused.clone(), used.clone(), untagged.clone()];
    h.settle_ui(5500);
    let session = h.observe(|v| {
        (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation,
        )
    });
    h.drain_commands();
    h.click("0-1"); // Kit's Workspace group, Images sidebar item.
    assert!(h.observe(|v| v.images_open));
    let mut generation = h.observe(|v| v.images.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::Images { generation: current } if *current == generation)));
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.checkpoint("images-inventory");
    verify_image_details(h, &inventory);
    verify_image_prune(h, &inventory);
    generation = h.observe(|v| v.images.generation);
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let row = window.find("image-row-aaaaaaaaaaaa").bounds();
        let size = window.find("image-size-aaaaaaaaaaaa").bounds();
        assert!(
            window.find("image-sort-name").bounds().size.width < px(200.0),
            "Image sort header must stay aligned with the image names"
        );
        assert!(
            size.origin.x > row.origin.x + row.size.width / 2.0,
            "Image sizes must align in their own column"
        );
        assert!(
            size.origin.y >= row.origin.y && size.bottom() <= row.bottom(),
            "Image metadata must stay within its rendered row"
        );
    })
    .unwrap();
    verify_image_sorting(h, &inventory, generation);
    verify_image_multiselection(h, &inventory);
    generation = h.observe(|v| v.images.generation);
    h.click("image-row-bbbbbbbbbbbb");
    h.click("image-remove");
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveImage { .. }))
    );
    h.checkpoint("images-stopped-reference");
    h.key("ctrl-f");
    h.type_text("FERRYDOCK-DEMO");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    h.checkpoint("images-search");
    h.key("ctrl-a");
    h.type_text("no-such-image");
    assert!(h.observe(|v| v.images.visible.is_empty()));
    h.checkpoint("images-no-matches");
    h.key("ctrl-a");
    h.type_text("cccccccccccc");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    h.key("escape");
    h.key("down");
    assert_eq!(
        h.observe(|v| v.images.selected.clone()),
        Some(untagged.id.clone())
    );
    h.checkpoint("images-untagged-unavailable-size");
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("image-row-aaaaaaaaaaaa");
    h.click("image-remove");
    assert_eq!(
        h.observe(|v| v.images.confirmation.clone()),
        Some(vec![unused.id.clone()])
    );
    h.checkpoint("images-removal-review");
    assert!(
        h.cx.update_window(h.window.into(), |_, window, cx| window
            .has_active_dialog(cx))
            .unwrap()
    );
    h.click("image-cancel-remove");
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    h.click("image-remove");
    h.key("escape");
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    h.click("image-remove");
    // A newly attached stopped container cancels an already open review.
    let mut referenced = unused.clone();
    referenced.containers = used.containers.clone();
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![referenced, used.clone(), untagged.clone()]),
    });
    assert!(h.observe(|v| v.images.confirmation.is_none()));
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("image-remove");
    h.click("image-confirm-remove");
    let commands = h.drain_commands();
    let operation = commands
        .iter()
        .find_map(|command| match command {
            Command::RemoveImage { id, operation } if *id == unused.id => Some(*operation),
            _ => None,
        })
        .expect("Confirmed image removal must send the reviewed immutable ID");
    h.click("image-remove");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveImage { .. }))
    );
    h.checkpoint("images-removing");
    h.deliver(Event::ImageRemoved {
        id: unused.id.clone(),
        operation,
        result: Err("Docker refused removal: image has multiple tags".into()),
    });
    h.checkpoint("images-removal-failed");
    h.dismiss_image_feedback();
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: generation - 1,
        result: Ok(vec![]),
    });
    assert_eq!(h.observe(|v| v.images.items.len()), 3);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.click("image-remove");
    h.click("image-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveImage { id, operation } if *id == unused.id => Some(*operation),
            _ => None,
        })
        .unwrap();
    h.deliver(Event::ImageRemoved {
        id: unused.id.clone(),
        operation,
        result: Ok(()),
    });
    assert!(!h.observe(|v| v.images.items.iter().any(|image| image.id == unused.id)));
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![used.clone(), untagged.clone()]),
    });
    h.checkpoint("images-removed");
    h.dismiss_image_feedback();
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    assert!(h.observe(|v| v.images_open && !v.settings_open));
    h.checkpoint("images-light");
    h.click("image-prune");
    h.checkpoint("images-prune-light");
    h.click("image-prune-cancel");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    let many = (0..10_000)
        .map(|index| Image {
            id: format!("sha256:{index:064x}"),
            tags: vec![format!("fixture-image-{index:05}:latest")],
            ..unused.clone()
        })
        .collect();
    h.deliver(Event::Images {
        generation,
        result: Ok(many),
    });
    h.checkpoint("images-large-inventory");
    let selection = h.observe(|v| v.images.selection.clone());
    h.drag_list_scrollbar("images-viewport", true);
    assert!(h.observe(|v| v.images.scroll.0.borrow().base_handle.offset().y < px(-1000.0)));
    assert_eq!(h.observe(|v| v.images.selection.clone()), selection);
    h.checkpoint("images-scrollbar-dragged");
    h.drag_list_scrollbar("images-viewport", false);
    assert!(h.observe(|v| v.images.scroll.0.borrow().base_handle.offset().y > px(-100.0)));
    assert!(
        h.observe(|v| v.images.rows_rendered.get()) < 40,
        "Image list must virtualize a large inventory"
    );
    h.key("ctrl-f");
    h.type_text("fixture-image-09999");
    h.key("escape");
    h.key("down");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    assert_eq!(
        h.observe(|v| v.images.selected.clone()),
        Some(format!("sha256:{:064x}", 9999))
    );
    h.checkpoint("images-large-search");
    h.click("0-0");
    assert!(!h.observe(|v| v.images_open));
    assert_eq!(
        session,
        h.observe(|v| (
            v.containers.selected.clone(),
            v.container_logs.generation,
            v.terminal.generation,
            v.project_logs.generation
        ))
    );
    h.click("0-1");
    let generation = h.observe(|v| v.images.generation);
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![unused.clone(), used.clone(), untagged.clone()]),
    });
    h.click("image-row-bbbbbbbbbbbb");
    h.click("image-container-fixture-probe-stopped");
    assert!(!h.observe(|v| v.images_open));
    assert!(!h.observe(|v| v.containers.running_only));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        Some("fixture-probe-stopped".into())
    );
    h.checkpoint("images-reference-navigation");
    h.click("0-1");
    h.drain_commands();
    h.key("ctrl-r");
    let generation = h.observe(|v| v.images.generation);
    let commands = h.drain_commands();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::Refresh))
    );
    assert!(commands.iter().any(|command| matches!(command, Command::Images { generation: current } if *current == generation)));
    h.deliver(Event::Images {
        generation,
        result: Err("Fixture inventory failure".into()),
    });
    h.checkpoint("images-discovery-failed");
    h.click("image-prune");
    assert!(!h.observe(|v| v.images.prune_review));
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![unused.clone(), used, untagged]),
    });
    h.click("image-row-aaaaaaaaaaaa");
    h.click("image-remove");
    h.click("image-confirm-remove");
    let operation = h.observe(|v| v.images.busy[&unused.id]);
    h.click_tab("image-tabs", 1);
    let detail_generation = h.observe(|v| v.images.details.generation);
    h.deliver(Event::Disconnected("Fixture Docker disconnect".into()));
    h.deliver(Event::ImageHistory {
        id: unused.id.clone(),
        generation: detail_generation,
        result: Ok(vec![]),
    });
    h.deliver(Event::ImageInspection {
        id: unused.id.clone(),
        generation: detail_generation,
        result: Ok("stale disconnected inspection".into()),
    });
    assert!(
        h.observe(|v| v.images.details.history.is_none() && v.images.details.inspection.is_none())
    );
    h.checkpoint("images-history-disconnected");
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.deliver(Event::ImageRemoved {
        id: unused.id,
        operation,
        result: Ok(()),
    });
    assert!(!h.observe(|v| v.images.loaded));
    assert!(h.observe(|v| v.images.busy.is_empty()));
    assert!(h.observe(|v| {
        v.images
            .items
            .iter()
            .any(|image| image.id == format!("sha256:{}", "a".repeat(64)))
    }));
    h.checkpoint("images-disconnected");
    h.deliver(Event::Connected(connection.clone()));
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::ImageDetails { id, .. } if id == &format!("sha256:{}", "a".repeat(64)))), "Reconnect reloads active image details");
    h.key("ctrl-1");
    h.click("image-prune");
    assert!(h.observe(|v| v.images.prune_review));
    h.deliver(Event::Disconnected(
        "Fixture disconnected during prune review".into(),
    ));
    assert!(!h.observe(|v| v.images.prune_review));
    assert!(
        !h.cx
            .update_window(h.window.into(), |_, window, cx| window
                .has_active_dialog(cx))
            .unwrap()
    );
    h.deliver(Event::Connected(connection));
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory),
    });
    h.click("image-prune");
    h.click("image-prune-unused");
    let operation = h.observe(|v| v.images.pruning.unwrap());
    h.deliver(Event::Disconnected(
        "Fixture disconnected during pruning".into(),
    ));
    h.drain_commands();
    h.deliver(Event::ImagesPruned {
        operation,
        result: Ok(Some(1_048_576)),
    });
    assert!(h.observe(|v| v.images.pruning.is_none() && !v.images.loaded));
    assert!(
        h.drain_commands().is_empty(),
        "Disconnected prune completion is ignored"
    );
    h.checkpoint("images-prune-disconnected");
}
