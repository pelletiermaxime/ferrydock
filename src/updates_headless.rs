//! Updater controls driven through GPUI input with a fixture worker boundary.
use super::*;
use crate::updater::{Command as UpdateCommand, Event as UpdateEvent, Worker};

#[test]
fn headless_gpu_update_flow() {
    let started = Instant::now();
    let mut h = Harness::new();
    let (worker, commands, events) = Worker::fixture();
    // Wire the fixture boundary before any input; behavior below uses only
    // painted controls and worker responses, never private action handlers.
    h.view.update(&mut h.cx, |view, _| {
        view.updates = super::super::updates::Updates::fixture(worker);
    });
    h.deliver(Event::Connected(Connection {
        name: "Updater fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.deliver(Event::Snapshot(vec![container(
        "updater-probe",
        "updater-probe",
        "running",
        None,
    )]));
    h.click("row-updater-probe");
    h.key("enter");
    let generation = h.observe(|v| v.terminal.generation);
    assert!(
        h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Terminal { .. }))
    );
    h.deliver(Event::TerminalReady { generation });
    let deliver = |h: &mut Harness, event| {
        events.try_send(event).unwrap();
        h.cx.advance_clock(Duration::from_millis(40));
        h.cx.run_until_parked();
    };
    deliver(
        &mut h,
        UpdateEvent::Loaded {
            automatic: true,
            unavailable: None,
        },
    );
    h.key("ctrl-,");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.within("settings-sidebar").click("0-1", cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.capture("updates-settings");
    h.click("update-automatic");
    assert!(matches!(
        commands.try_recv(),
        Ok(UpdateCommand::Automatic(false))
    ));
    deliver(&mut h, UpdateEvent::Automatic(false));
    h.click("update-check");
    assert!(matches!(commands.try_recv(), Ok(UpdateCommand::Check)));
    h.click("update-check");
    assert!(
        commands.try_recv().is_err(),
        "Busy check must prevent duplicate requests"
    );
    deliver(&mut h, UpdateEvent::Checking);
    deliver(
        &mut h,
        UpdateEvent::Downloading {
            version: "0.13.0".into(),
            bytes: 50 * 1_048_576,
            total: 100 * 1_048_576,
        },
    );
    h.capture("updates-downloading");
    deliver(&mut h, UpdateEvent::Ready("0.13.0".into()));
    h.capture("updates-ready");
    assert_eq!(
        h.observe(|v| v.terminal.generation),
        generation,
        "Downloading must preserve the live shell"
    );
    assert!(
        commands.try_recv().is_err(),
        "A downloaded update must never apply automatically"
    );
    h.click("update-restart");
    h.capture("updates-restart-review");
    h.deliver(Event::ActionStarted {
        id: "updater-probe".into(),
        action: Action::Restart,
    });
    h.click("update-restart-confirm");
    assert!(
        commands.try_recv().is_err(),
        "A newly started Docker mutation must block a stale restart review"
    );
    h.deliver(Event::ActionFinished {
        id: "updater-probe".into(),
        result: Ok(()),
    });
    h.click("update-restart-cancel");
    assert!(
        commands.try_recv().is_err(),
        "Later must leave the downloaded update staged"
    );
    h.click("update-restart");
    h.click("update-restart-confirm");
    assert!(matches!(commands.try_recv(), Ok(UpdateCommand::Apply)));
    h.key("ctrl-k");
    assert!(
        !h.observe(|v| v.quick_surface_open()),
        "Applying must prevent opening another terminal"
    );
    h.capture("updates-applying");
    deliver(
        &mut h,
        UpdateEvent::Error("Could not restart; restored the previous AppImage".into()),
    );
    h.capture("updates-restart-failed");
    assert!(!h.observe(|v| v.updates.applying));
    h.click("update-check");
    assert!(matches!(commands.try_recv(), Ok(UpdateCommand::Check)));
    deliver(&mut h, UpdateEvent::Current);
    h.capture("updates-current");
    deliver(
        &mut h,
        UpdateEvent::Loaded {
            automatic: true,
            unavailable: Some("In-app installation is available for AppImages.".into()),
        },
    );
    h.click("update-check");
    assert!(
        commands.try_recv().is_err(),
        "A package-managed installation must never self-replace"
    );
    h.capture("updates-other-installation");
    // Exercise the same Kit settings controls in Light at the minimum width.
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("settings-sidebar").click("0-0", cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.click("theme-light");
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("settings-sidebar").click("0-1", cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.capture("updates-light");
    let mut narrow = Harness::with_size(820.0, 640.0);
    let (worker, _commands, events) = Worker::fixture();
    narrow.view.update(&mut narrow.cx, |view, _| {
        view.updates = super::super::updates::Updates::fixture(worker);
    });
    events
        .try_send(UpdateEvent::Loaded {
            automatic: true,
            unavailable: None,
        })
        .unwrap();
    narrow.cx.advance_clock(Duration::from_millis(40));
    narrow.cx.run_until_parked();
    narrow.key("ctrl-,");
    narrow
        .cx
        .update_window(narrow.window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.within("settings-sidebar").click("0-1", cx);
        })
        .unwrap();
    narrow.cx.run_until_parked();
    narrow.capture("updates-light-narrow");
    narrow
        .cx
        .update_window(narrow.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let checkbox = window.find("update-automatic").bounds();
            assert!(
                checkbox.right() <= window.viewport_size().width,
                "Update controls must fit the window"
            );
        })
        .unwrap();
    h.screenshots.extend(narrow.screenshots.clone());
    h.frame_timings.extend(narrow.frame_timings.clone());
    h.actions.extend(narrow.actions.clone());
    std::fs::write(h.evidence.join("headless.json"), serde_json::to_vec_pretty(&json!({
        "status": "passed", "renderer": "GPUI Kit gpui-fast WgpuHeadlessRenderer",
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820, "pixel_width": 2360, "pixel_height": 1640,
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "screenshots": h.screenshots, "frame_timings": h.frame_timings,
        "checks": ["Settings navigation", "automatic preference request", "manual check and duplicate guard", "download progress in MiB", "download preserves live shell", "ready banner without automatic apply", "restart review and Later", "new Docker mutation blocks stale restart review", "manual apply request", "apply blocks new terminals", "failed restart recovers and retries", "current release", "non-AppImage installation protected", "Light theme at minimum width"]
    })).unwrap()).unwrap();
    std::fs::write(
        h.evidence.join("actions.json"),
        serde_json::to_vec_pretty(&h.actions).unwrap(),
    )
    .unwrap();
}
