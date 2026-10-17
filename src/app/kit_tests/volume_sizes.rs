use super::*;

pub(super) fn verify_volume_sizes(h: &mut Harness, inventory: &[crate::volumes::Volume]) {
    use crate::volumes::VolumeSize;
    let measured = |sizes: [Option<u64>; 3]| {
        inventory
            .iter()
            .zip(sizes)
            .map(|(volume, size)| VolumeSize {
                name: volume.name.clone(),
                driver: volume.driver.clone(),
                mountpoint: volume.mountpoint.clone(),
                created: volume.created.clone(),
                size,
            })
            .collect::<Vec<_>>()
    };
    let values = |h: &Harness| {
        h.observe(|v| {
            v.volumes
                .items
                .iter()
                .map(|volume| volume.size)
                .collect::<Vec<_>>()
        })
    };
    let order = |h: &Harness| h.observe(|v| v.volumes.visible.clone());
    let no_gather = |commands: Vec<Command>| {
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::GatherVolumeSizes { .. })),
            "Refresh/navigation must never recalculate sizes"
        )
    };
    h.drain_commands();
    assert_eq!(values(h), vec![None; 3]);
    h.ctrl_click("volume-row-demo-database");
    let selection = h.observe(|v| v.volumes.selection.clone());
    h.click("volume-gather-sizes");
    let commands = h.drain_commands();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::GatherVolumeSizes { request } => Some(*request),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        commands
            .iter()
            .filter(|command| matches!(command, Command::GatherVolumeSizes { .. }))
            .count(),
        1
    );
    h.click("volume-gather-sizes");
    no_gather(h.drain_commands());
    h.checkpoint("volumes-gathering-sizes");
    // Ordinary discovery continues while the independently queued calculation runs.
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
    h.deliver(Event::VolumeSizes {
        request: request + 1,
        result: Ok(measured([Some(999); 3])),
    });
    assert!(h.observe(|v| v.volumes.gathering == Some(request)));
    assert_eq!(values(h), vec![None; 3]);
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(measured([Some(10 * 1_048_576), Some(2 * 1_048_576), None])),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    assert!(h.observe(|v| v.volumes.gathering.is_none()));
    h.click("volume-sort-size");
    assert_eq!(order(h), vec![1, 0, 2]);
    h.checkpoint("volumes-sizes-smallest-first");
    h.cx.update_window(h.window.into(), |_, window, _cx| {
        let size = window.find("volume-size-demo-cache").bounds();
        let header = window.find("volume-sort-size").bounds();
        let row = window.find("volume-row-demo-cache").bounds();
        assert!((size.origin.x - header.origin.x).abs() < px(2.0));
        assert!(size.origin.y >= row.origin.y && size.bottom() <= row.bottom());
    })
    .unwrap();
    h.click("volume-sort-size");
    assert_eq!(order(h), vec![0, 1, 2]);
    assert_eq!(h.observe(|v| v.volumes.selection.clone()), selection);
    h.checkpoint("volumes-sizes-largest-first");
    no_gather(h.drain_commands());
    let mut refresh = inventory.to_vec();
    for volume in &mut refresh {
        volume.size = Some(123);
    }
    h.key("ctrl-r");
    no_gather(h.drain_commands());
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(refresh.clone()),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    h.deliver(Event::Volumes {
        generation,
        result: Err("Fixture inventory failure".into()),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    h.click("0-0");
    h.click("0-2");
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
    no_gather(h.drain_commands());
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    // Failure preserves the last measurements; only another explicit click replaces them.
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    h.drain_commands();
    h.deliver(Event::VolumeSizes {
        request,
        result: Err("Fixture Docker disk usage failure".into()),
    });
    assert_eq!(
        values(h),
        vec![Some(10 * 1_048_576), Some(2 * 1_048_576), None]
    );
    h.checkpoint("volumes-size-error-keeps-measurements");
    h.click("volume-gather-sizes");
    let request = h.observe(|v| v.volumes.gathering.unwrap());
    h.drain_commands();
    h.click("0-0");
    h.deliver(Event::VolumeSizes {
        request,
        result: Ok(measured([Some(0), Some(20 * 1_048_576), None])),
    });
    h.click("0-2");
    let generation = h.observe(|v| v.volumes.generation);
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
    assert_eq!(values(h), vec![Some(0), Some(20 * 1_048_576), None]);
    assert_eq!(order(h), vec![1, 0, 2]);
    no_gather(h.drain_commands());
    h.checkpoint("volumes-sizes-regathered-after-workspace-change");
    refresh[1].created = Some("2026-10-14T00:00:00Z".into());
    h.deliver(Event::Volumes {
        generation,
        result: Ok(refresh),
    });
    assert_eq!(
        values(h),
        vec![Some(0), None, None],
        "Recreated volumes must not inherit a cached size"
    );
    h.click("volume-sort-name");
    h.click("volume-row-demo-cache");
    h.deliver(Event::Volumes {
        generation,
        result: Ok(inventory.to_vec()),
    });
}
