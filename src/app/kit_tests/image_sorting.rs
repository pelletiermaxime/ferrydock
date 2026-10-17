use super::*;

pub(super) fn verify_image_sorting(
    h: &mut Harness,
    inventory: &[crate::images::Image],
    generation: u64,
) {
    let selected = h.observe(|v| v.images.selected.clone());
    let assert_order = |h: &Harness, expected: &[usize]| {
        let ids = h.observe(|v| {
            v.images
                .visible
                .iter()
                .map(|index| v.images.items[*index].id.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            ids,
            expected
                .iter()
                .map(|index| inventory[*index].id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            h.observe(|v| v.images.selected.clone()),
            selected,
            "Sorting must preserve the selected image ID"
        );
    };
    h.drain_commands();
    h.click("image-sort-size");
    assert_order(h, &[1, 0, 2]); // 32 MiB before 128 MiB; unavailable stays last.
    h.checkpoint("images-size-ascending");
    h.cx.update_window(h.window.into(), |_, window, _cx| {
        let small = window.find("image-row-bbbbbbbbbbbb").bounds();
        let large = window.find("image-row-aaaaaaaaaaaa").bounds();
        let missing = window.find("image-row-cccccccccccc").bounds();
        assert!(
            small.origin.y < large.origin.y && large.origin.y < missing.origin.y,
            "Sorted image order must reach the rendered rows"
        );
    })
    .unwrap();
    h.click("image-sort-size");
    assert_order(h, &[0, 1, 2]);
    h.checkpoint("images-size-descending");
    let mut refreshed = inventory.to_vec();
    refreshed.reverse();
    h.deliver(Event::Images {
        generation,
        result: Ok(refreshed),
    });
    assert_order(h, &[0, 1, 2]);
    h.click("image-sort-age");
    assert_order(h, &[0, 1, 2]);
    h.checkpoint("images-age-youngest-first");
    h.click("image-sort-age");
    assert_order(h, &[1, 0, 2]);
    h.checkpoint("images-age-oldest-first");
    h.cx.update_window(h.window.into(), |_, window, _cx| {
        let created = window.find("image-age-aaaaaaaaaaaa").bounds();
        let size = window.find("image-size-aaaaaaaaaaaa").bounds();
        assert!(
            created.origin.x < size.origin.x
                && created.origin.y >= window.find("image-row-aaaaaaaaaaaa").bounds().origin.y,
            "Image ages must render in their own aligned column"
        );
    })
    .unwrap();
    h.click("image-sort-age");
    assert_order(h, &[0, 1, 2]);
    h.key("ctrl-f");
    h.type_text("postgres");
    assert_order(h, &[1]);
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    assert_order(h, &[0, 1, 2]);
    h.click("image-sort-references");
    assert_order(h, &[0, 2, 1]);
    h.click("image-sort-references");
    assert_order(h, &[1, 0, 2]);
    h.checkpoint("images-references-descending");
    h.click("image-sort-name");
    assert_order(h, &[0, 1, 2]);
    h.click("image-sort-name");
    assert_order(h, &[2, 1, 0]);
    h.checkpoint("images-name-descending");
    h.click("image-sort-name");
    assert_order(h, &[0, 1, 2]);
    assert!(
        h.drain_commands().is_empty(),
        "Sorting must not restart worker sessions"
    );
}

pub(super) fn verify_image_multiselection(h: &mut Harness, inventory: &[crate::images::Image]) {
    use crate::images::RemovalResult;
    let ids = inventory
        .iter()
        .map(|image| image.id.clone())
        .collect::<Vec<_>>();
    h.click("image-row-aaaaaaaaaaaa");
    h.ctrl_click("image-row-cccccccccccc");
    assert_eq!(
        h.observe(|v| v.images.selection.iter().cloned().collect::<Vec<_>>()),
        vec![ids[0].clone(), ids[2].clone()]
    );
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("image-row-aaaaaaaaaaaa").selected(), Some(true));
        assert_eq!(window.find("image-row-cccccccccccc").selected(), Some(true));
    })
    .unwrap();
    h.ctrl_click("image-row-cccccccccccc");
    h.ctrl_click("image-row-aaaaaaaaaaaa");
    assert!(h.observe(|v| v.images.selection.is_empty() && v.images.selected.is_none()));
    h.drain_commands();
    h.click("image-remove");
    assert!(h.drain_commands().is_empty());
    h.click("image-row-aaaaaaaaaaaa");
    h.ctrl_click("image-row-cccccccccccc");
    h.checkpoint("images-multiple-selected");
    h.click("image-sort-size");
    assert_eq!(h.observe(|v| v.images.selection.len()), 2);
    h.key("ctrl-f");
    h.type_text("ferrydock-demo");
    h.key("escape");
    assert_eq!(h.observe(|v| v.images.visible.len()), 1);
    let generation = h.observe(|v| v.images.generation);
    let mut refreshed = inventory.to_vec();
    refreshed.reverse();
    h.deliver(Event::Images {
        generation,
        result: Ok(refreshed),
    });
    assert_eq!(h.observe(|v| v.images.selection.len()), 2);
    h.checkpoint("images-multiple-hidden-selection");
    h.click("image-remove");
    assert_eq!(
        h.observe(|v| v.images.confirmation.clone()),
        Some(vec![ids[0].clone(), ids[2].clone()])
    );
    h.checkpoint("images-multiple-hidden-review");
    h.click("image-cancel-remove");
    assert!(h.drain_commands().is_empty());
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("image-sort-name");
    h.ctrl_click("image-row-bbbbbbbbbbbb");
    assert_eq!(h.observe(|v| v.images.selection.len()), 3);
    h.click("image-remove");
    h.checkpoint("images-multiple-removal-review");
    h.click("image-confirm-remove");
    let operation = h.drain_commands().iter().find_map(|command| match command {
        Command::RemoveImages { ids: selected, operation } if *selected == ids => Some(*operation), _ => None,
    }).expect("Bulk deletion must send exactly the reviewed immutable IDs, including hidden selections");
    assert_eq!(h.observe(|v| v.images.busy.len()), 3);
    h.click("image-remove");
    assert!(h.drain_commands().is_empty());
    h.checkpoint("images-multiple-removing");
    h.deliver(Event::ImagesRemoved {
        operation: operation - 1,
        results: vec![RemovalResult {
            id: ids[1].clone(),
            result: Ok(()),
        }],
    });
    assert_eq!(h.observe(|v| v.images.busy.len()), 3);
    h.deliver(Event::ImagesRemoved {
        operation,
        results: vec![
            RemovalResult {
                id: ids[0].clone(),
                result: Ok(()),
            },
            RemovalResult {
                id: ids[1].clone(),
                result: Err("Image is used by a stopped container".into()),
            },
            RemovalResult {
                id: ids[2].clone(),
                result: Err("Docker refused removal: image has multiple tags".into()),
            },
        ],
    });
    assert!(h.observe(|v| v.images.busy.is_empty()));
    assert!(!h.observe(|v| v.images.items.iter().any(|image| image.id == ids[0])));
    assert_eq!(
        h.observe(|v| v.images.selection.iter().cloned().collect::<Vec<_>>()),
        vec![ids[1].clone(), ids[2].clone()]
    );
    h.checkpoint("images-multiple-partial-results");
    h.cx.update_window(h.window.into(), |_, window, _cx| {
        assert!(window.try_find("image-removal-results").is_none());
    })
    .unwrap();
    h.dismiss_image_feedback();
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let detail = window.find("image-detail").bounds();
        assert!(
            detail.size.height >= px(260.0) && detail.bottom() <= px(820.0),
            "Deletion feedback must leave the detail panel inside the window"
        );
    })
    .unwrap();
    let generation = h.observe(|v| v.images.generation);
    let mut available = inventory[1].clone();
    available.containers = std::sync::Arc::from([]);
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![available, inventory[2].clone()]),
    });
    assert_eq!(h.observe(|v| v.images.selection.len()), 2);
    h.click("image-remove");
    assert!(
        h.observe(|v| v.images.confirmation.is_some()),
        "Failed images must remain reviewable after inventory refresh"
    );
    h.click("image-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveImages { operation, .. } => Some(*operation),
            _ => None,
        })
        .unwrap();
    h.deliver(Event::ImagesRemoved {
        operation,
        results: vec![
            RemovalResult {
                id: ids[1].clone(),
                result: Ok(()),
            },
            RemovalResult {
                id: ids[2].clone(),
                result: Ok(()),
            },
        ],
    });
    assert!(h.observe(|v| v.images.selection.is_empty()
        && v.images.selected.is_none()
        && v.images.items.is_empty()));
    h.checkpoint("images-multiple-removed");
    h.dismiss_image_feedback();
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.to_vec()),
    });
    h.click("image-row-aaaaaaaaaaaa");
    assert_eq!(h.observe(|v| v.images.selection.len()), 1);
    h.drain_commands();
}
