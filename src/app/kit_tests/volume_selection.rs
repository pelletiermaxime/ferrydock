use super::*;

pub(super) fn verify_volume_sorting_and_selection(
    h: &mut Harness,
    original: &[crate::volumes::Volume],
) {
    use crate::volumes::{RemovalResult, Volume};
    let now = chrono::Utc::now().timestamp();
    let mut inventory = original.to_vec();
    inventory[0].created = Some(
        chrono::DateTime::from_timestamp(now - 864000, 0)
            .unwrap()
            .to_rfc3339(),
    );
    inventory[1].created = Some(
        chrono::DateTime::from_timestamp(now - 172800, 0)
            .unwrap()
            .to_rfc3339(),
    );
    inventory[2].created = None;
    inventory.push(Volume {
        name: "unavailable".into(),
        created: Some("invalid".into()),
        ..original[0].clone()
    });
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.clone()),
    });
    h.drain_commands();
    let assert_order = |h: &Harness, expected: &[usize]| {
        assert_eq!(
            h.observe(|v| v
                .volumes
                .visible
                .iter()
                .map(|index| v.volumes.items[*index].name.clone())
                .collect::<Vec<_>>()),
            expected
                .iter()
                .map(|index| inventory[*index].name.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            h.observe(|v| v.volumes.selected.clone()),
            Some(inventory[0].name.clone())
        );
    };
    for (header, ascending, descending) in [
        ("volume-sort-age", vec![1, 0, 2, 3], vec![0, 1, 2, 3]),
        ("volume-sort-driver", vec![0, 1, 3, 2], vec![2, 0, 1, 3]),
        ("volume-sort-references", vec![0, 2, 3, 1], vec![1, 0, 2, 3]),
        ("volume-sort-name", vec![0, 1, 2, 3], vec![3, 2, 1, 0]),
    ] {
        h.click(header);
        assert_order(h, &ascending);
        if header == "volume-sort-age" {
            h.checkpoint("volumes-age-youngest-first");
        }
        h.click(header);
        assert_order(h, &descending);
        if header == "volume-sort-age" {
            h.checkpoint("volumes-age-oldest-first");
        }
    }
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let row = window.find("volume-row-demo-cache").bounds();
        let age = window.find("volume-age-demo-cache").bounds();
        let header = window.find("volume-sort-age").bounds();
        assert!(age.origin.x > row.origin.x + row.size.width / 2.0);
        assert!(age.origin.y >= row.origin.y && age.bottom() <= row.bottom());
        assert!((age.origin.x - header.origin.x).abs() < px(2.0));
        let cache = window.find("volume-row-demo-cache").bounds();
        let database = window.find("volume-row-demo-database").bounds();
        assert!(
            database.origin.y < cache.origin.y,
            "Painted rows must follow descending name order"
        );
    })
    .unwrap();
    assert!(
        h.drain_commands().is_empty(),
        "Sorting must not send Docker operations"
    );
    // Ctrl-click supports multiple selection and clearing the final row.
    h.ctrl_click("volume-row-demo-database");
    h.ctrl_click("volume-row-remote-data");
    assert_eq!(h.observe(|v| v.volumes.selection.len()), 3);
    h.checkpoint("volumes-multiple-selected");
    h.ctrl_click("volume-row-demo-cache");
    h.ctrl_click("volume-row-demo-database");
    h.ctrl_click("volume-row-remote-data");
    assert!(h.observe(|v| v.volumes.selection.is_empty()));
    h.click("volume-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.click("volume-row-demo-cache");
    h.ctrl_click("volume-row-demo-database");
    h.ctrl_click("volume-row-remote-data");
    let selected = h.observe(|v| v.volumes.selection.clone());
    h.click("volume-sort-driver");
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selected);
    h.key("ctrl-f");
    h.type_text("demo-cache");
    h.key("escape");
    assert_eq!(h.observe(|v| v.volumes.visible.len()), 1);
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selected);
    h.checkpoint("volumes-multiple-hidden-selection");
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![
            inventory[2].clone(),
            inventory[3].clone(),
            inventory[1].clone(),
            inventory[0].clone(),
        ]),
    });
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selected);
    h.click("volume-remove");
    let names = selected.iter().cloned().collect::<Vec<_>>();
    assert_eq!(
        h.observe(|v| v.volumes.confirmation.clone()),
        Some(names.clone())
    );
    h.checkpoint("volumes-multiple-hidden-review");
    for name in &names {
        assert!(
            h.cx.debug_bounds(h.window.into(), &format!("volume-removal-target-{name}"))
                .unwrap()
                .is_some()
        );
    }
    h.click("volume-cancel-remove");
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    assert!(!h.drain_commands().iter().any(|command| matches!(
        command,
        Command::RemoveVolume { .. } | Command::RemoveVolumes { .. }
    )));
    h.click("volume-remove");
    h.key("escape");
    h.settle_ui(300);
    assert!(h.observe(|v| v.volumes.confirmation.is_none()));
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h
        .drain_commands()
        .iter()
        .find_map(|command| match command {
            Command::RemoveVolumes {
                names: actual,
                operation,
            } if actual == &names => Some(*operation),
            _ => None,
        })
        .expect("Bulk removal must contain every reviewed name, including hidden selections");
    h.click("volume-remove");
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::RemoveVolumes { .. }))
    );
    h.deliver(Event::VolumesRemoved {
        operation: operation + 1,
        results: vec![RemovalResult {
            name: inventory[0].name.clone(),
            result: Ok(()),
        }],
    });
    assert!(h.observe(|v| v.volumes.busy.is_some()));
    h.deliver(Event::VolumesRemoved {
        operation,
        results: vec![
            RemovalResult {
                name: inventory[0].name.clone(),
                result: Ok(()),
            },
            RemovalResult {
                name: inventory[1].name.clone(),
                result: Err("Volume is used by a stopped container".into()),
            },
            RemovalResult {
                name: inventory[2].name.clone(),
                result: Err("Fixture Docker conflict".into()),
            },
        ],
    });
    assert_eq!(
        h.observe(|v| v.volumes.selection.clone()),
        [inventory[1].name.clone(), inventory[2].name.clone()]
            .into_iter()
            .collect()
    );
    assert!(!h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == inventory[0].name)
    }));
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory[1..].to_vec()),
    });
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.settle_ui(300);
    h.checkpoint("volumes-multiple-partial-results");
    h.dismiss_image_feedback();
    h.click("volume-row-remote-data");
    h.ctrl_click("volume-row-unavailable");
    h.click("volume-remove");
    h.click("volume-confirm-remove");
    let operation = h.observe(|v| v.volumes.busy.as_ref().unwrap().1);
    h.deliver(Event::VolumesRemoved {
        operation,
        results: vec![
            RemovalResult {
                name: inventory[2].name.clone(),
                result: Ok(()),
            },
            RemovalResult {
                name: inventory[3].name.clone(),
                result: Ok(()),
            },
        ],
    });
    assert!(h.observe(|v| v.volumes.selection.is_empty() && v.volumes.selected.is_none()));
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(vec![inventory[1].clone()]),
    });
    assert!(h.observe(|v| v.volumes.selection.is_empty()));
    h.checkpoint("volumes-multiple-removed");
    h.dismiss_image_feedback();
    // Restore the ordinary single-selection flow through the same front door.
    h.deliver(Event::Volumes {
        generation,
        result: Ok(original.to_vec()),
    });
    h.click("volume-sort-name");
    h.click("volume-row-demo-cache");
}
