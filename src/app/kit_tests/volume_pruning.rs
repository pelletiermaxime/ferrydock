use super::*;

pub(super) fn verify_volume_pruning(h: &mut Harness, original: &[crate::volumes::Volume]) {
    use crate::volumes::{PruneMode, PruneResult, Volume, VolumeSize};
    let connection = Connection {
        name: "headless-fixture".into(),
        endpoint: "unix:///headless-fixture.sock".into(),
    };
    h.deliver(Event::Connected(connection.clone()));
    let mut inventory = original.to_vec();
    inventory.push(Volume {
        name: "hidden-unused".into(),
        ..original[0].clone()
    });
    let deliver = |h: &mut Harness, inventory: &[Volume]| {
        let generation = h.observe(|v| v.volumes.generation);
        h.deliver(Event::Volumes {
            generation,
            result: Ok(inventory.to_vec()),
        });
    };
    deliver(h, &inventory);
    h.click("volume-row-demo-cache");
    h.drain_commands();
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    let measurements = inventory
        .iter()
        .enumerate()
        .map(|(index, volume)| VolumeSize {
            name: volume.name.clone(),
            driver: volume.driver.clone(),
            mountpoint: volume.mountpoint.clone(),
            created: volume.created.clone(),
            size: Some(index as u64 * 1_048_576),
        })
        .collect();
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(measurements),
    });
    h.ctrl_click("volume-row-demo-cache");
    assert!(h.observe(|v| v.volumes.selection.is_empty()));
    h.key("ctrl-f");
    h.type_text("demo-database");
    h.key("escape");
    assert_eq!(h.observe(|v| v.volumes.visible.len()), 1);
    h.drain_commands();
    h.click("volume-prune");
    assert!(h.observe(|v| v.volumes.prune_review));
    h.checkpoint("volumes-prune-review-all-unused");
    h.key("enter");
    assert!(h.observe(|v| v.volumes.prune_review && v.volumes.pruning.is_none()));
    h.click("volume-prune-cancel");
    assert!(!h.observe(|v| v.volumes.prune_review));
    h.click("volume-prune");
    h.key("escape");
    assert!(!h.observe(|v| v.volumes.prune_review));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::PruneVolumes { .. }))
    );
    // A conflicting worker action disables prune, even with a loaded inventory.
    h.deliver(Event::ActionStarted {
        id: "fixture-probe-running".into(),
        action: Action::Stop,
    });
    h.click("volume-prune");
    assert!(!h.observe(|v| v.volumes.prune_review));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::PruneVolumes { .. }))
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-running".into(),
        result: Ok(()),
    });
    deliver(h, &inventory);
    h.dismiss_image_feedback();
    for (index, result) in [
        Err("Fixture Docker prune failure".into()),
        Ok(PruneResult {
            deleted: vec![original[0].name.clone(), "hidden-unused".into()],
            reclaimed: Some(3 * 1_048_576),
        }),
        Ok(PruneResult {
            deleted: vec![],
            reclaimed: None,
        }),
        Ok(PruneResult {
            deleted: vec![],
            reclaimed: Some(0),
        }),
    ]
    .into_iter()
    .enumerate()
    {
        let mode = if index == 0 || index == 2 {
            PruneMode::Anonymous
        } else {
            PruneMode::AllUnused
        };
        if index == 1 {
            h.click("volume-row-demo-database");
            h.key("ctrl-f");
            h.key("ctrl-a");
            h.type_text("demo-cache");
            h.key("escape");
            h.ctrl_click("volume-row-demo-cache");
            h.key("ctrl-f");
            h.key("ctrl-a");
            h.type_text("demo-database");
            h.key("escape");
        }
        h.click("volume-prune");
        h.click(if mode == PruneMode::Anonymous {
            "volume-prune-anonymous"
        } else {
            "volume-prune-unused"
        });
        let operation = h.observe(|v| v.volumes.pruning.unwrap());
        assert!(!h.observe(|v| v.volumes.prune_review));
        assert_eq!(h.drain_commands().iter().filter(|command| matches!(command, Command::PruneVolumes { mode: current_mode, operation: current } if *current == operation && *current_mode == mode)).count(), 1);
        h.click("volume-prune");
        h.click("volume-remove");
        assert!(!h.drain_commands().iter().any(|command| matches!(
            command,
            Command::PruneVolumes { .. }
                | Command::RemoveVolume { .. }
                | Command::RemoveVolumes { .. }
        )));
        h.deliver(Event::VolumesPruned {
            operation: operation + 1,
            result: Ok(PruneResult {
                deleted: vec![],
                reclaimed: Some(999),
            }),
        });
        assert_eq!(h.observe(|v| v.volumes.pruning), Some(operation));
        if index == 0 {
            h.checkpoint("volumes-prune-pending");
            h.click("0-0");
            h.drain_commands();
            h.click("primary-action");
            assert!(
                !h.drain_commands()
                    .iter()
                    .any(|command| matches!(command, Command::Action { .. }))
            );
            h.click("0-1");
            let generation = h.observe(|v| v.images.generation);
            h.deliver(Event::Images {
                generation,
                result: Ok(vec![]),
            });
            h.drain_commands();
            h.click("image-prune");
            assert!(!h.observe(|v| v.images.prune_review));
            assert!(
                !h.drain_commands()
                    .iter()
                    .any(|command| matches!(command, Command::PruneImages { .. }))
            );
            h.click("0-2");
            deliver(h, &inventory);
        }
        if index == 2 {
            h.click("0-0");
        }
        h.deliver(Event::VolumesPruned { operation, result });
        assert!(h.observe(|v| v.volumes.pruning.is_none()));
        h.settle_ui(300);
        assert!(h.notification_count() > 0);
        if index == 0 {
            assert_eq!(h.observe(|v| v.volumes.items.len()), 4);
            h.checkpoint("volumes-prune-failed-keeps-measurements");
        } else {
            assert_eq!(h.observe(|v| v.volumes.items.len()), 2);
            assert_eq!(
                h.observe(|v| v.volumes.selection.clone()),
                [original[1].name.clone()].into_iter().collect()
            );
            assert_eq!(
                h.observe(|v| v
                    .volumes
                    .items
                    .iter()
                    .find(|volume| volume.name == original[1].name)
                    .unwrap()
                    .size),
                Some(1_048_576)
            );
            inventory = original[1..].to_vec();
            if index == 1 {
                h.checkpoint("volumes-prune-success");
            } else {
                h.click("0-2");
                h.checkpoint(if index == 2 {
                    "volumes-prune-space-unavailable"
                } else {
                    "volumes-prune-zero-reclaimed"
                });
            }
        }
        let commands = h.drain_commands();
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::GatherVolumeSizes { .. })),
            "Pruning never regathers surviving volume sizes"
        );
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Volumes { .. })),
            "Pruning reconciles the inventory"
        );
        h.dismiss_image_feedback();
        deliver(h, &inventory);
        assert_eq!(
            h.observe(|v| v
                .volumes
                .items
                .iter()
                .find(|volume| volume.name == original[1].name)
                .unwrap()
                .size),
            Some(1_048_576)
        );
    }
    h.click("volume-prune");
    h.click("volume-prune-unused");
    let operation = h.observe(|v| v.volumes.pruning.unwrap());
    h.deliver(Event::Disconnected("Fixture prune disconnect".into()));
    h.deliver(Event::VolumesPruned {
        operation,
        result: Ok(PruneResult {
            deleted: vec![original[1].name.clone()],
            reclaimed: Some(1_048_576),
        }),
    });
    assert!(h.observe(|v| v.volumes.pruning.is_none() && !v.volumes.prune_review));
    assert!(h.observe(|v| {
        v.volumes
            .items
            .iter()
            .any(|volume| volume.name == original[1].name)
    }));
    h.checkpoint("volumes-prune-disconnected");
    h.dismiss_image_feedback();
    h.deliver(Event::Connected(connection));
    deliver(h, original);
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    h.click("volume-row-demo-cache");
    h.drain_commands();
}
