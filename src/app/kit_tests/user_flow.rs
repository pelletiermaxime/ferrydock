use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let started = Instant::now();
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Headless fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.deliver(Event::Snapshot(vec![
        container(
            "fixture-probe-running",
            "headless-fixture-probe-1",
            "running",
            Some("headless-fixture"),
        ),
        container(
            "fixture-probe-stopped",
            "headless-fixture-probe-2",
            "exited",
            Some("headless-fixture"),
        ),
        container("fixture-standalone", "standalone-example", "running", None),
    ]));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    let original_containers = h.observe(|v| v.containers.items.clone());
    h.click("all");
    h.checkpoint("compose-groups-expanded");
    h.click("all");
    h.drain_commands();
    h.click("toggle-compose-headless-fixture");
    h.checkpoint("compose-group-collapsed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    assert!(h.observe(|v| v.projects.selected.is_none()));
    assert!(
        h.drain_commands().is_empty(),
        "Folding must preserve the detail session"
    );
    h.deliver(Event::Snapshot(original_containers.clone()));
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );

    // Search temporarily reveals folded matches without discarding the fold.
    h.key("ctrl-f");
    h.type_text("headless-fixture");
    h.checkpoint("compose-group-search-revealed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_some()
    );
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );
    h.key("down");
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-standalone")
    );
    h.click("toggle-standalone");
    h.checkpoint("all-container-groups-collapsed");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_none()
    );
    h.key("up");
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-standalone")
    );
    h.click("toggle-standalone");
    h.click("toggle-compose-headless-fixture");
    h.click("row-fixture-probe-running");
    let mut many_containers = original_containers.clone();
    many_containers.extend((0..100).map(|index| {
        container(
            &format!("scroll-fixture-{index:03}"),
            &format!("scroll-fixture-{index:03}"),
            "running",
            Some("scroll-fixture"),
        )
    }));
    h.deliver(Event::Snapshot(many_containers));
    h.checkpoint("containers-scrollbar-top");
    h.drag_list_scrollbar("containers-viewport", true);
    assert!(h.observe(|v| v.containers.scroll.0.borrow().base_handle.offset().y < px(-1000.0)));
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()).as_deref(),
        Some("fixture-probe-running")
    );
    h.checkpoint("containers-scrollbar-dragged");
    h.drag_list_scrollbar("containers-viewport", false);
    assert!(h.observe(|v| v.containers.scroll.0.borrow().base_handle.offset().y > px(-100.0)));
    h.deliver(Event::Snapshot(original_containers));
    h.deliver(Event::MetricsSession(1));
    h.checkpoint("metrics-collecting");
    for index in 0..60 {
        h.deliver(Event::Metrics {
            session: 1,
            id: "fixture-probe-running".into(),
            result: Ok(crate::metrics::Sample {
                collected: Instant::now() - Duration::from_secs((59 - index) * 2),
                cpu_percent: Some(12.0 + ((index * 7) % 23) as f64 * 8.0),
                memory_used: Some((144 + (index % 12) * 4) * 1024 * 1024),
                memory_capacity: Some(512 * 1024 * 1024),
                limits: Some(crate::metrics::Limits {
                    memory: Some(512 * 1024 * 1024),
                    cpu_cores: Some(2.0),
                    cpuset: None,
                }),
                reset: false,
            }),
        });
    }
    let last_sample = h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .samples
            .back()
            .unwrap()
            .clone()
    });
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-standalone".into(),
        result: Ok(crate::metrics::Sample {
            limits: Some(crate::metrics::Limits::default()),
            memory_capacity: Some(8 * 1024 * 1024 * 1024),
            ..last_sample.clone()
        }),
    });
    h.deliver(Event::Metrics {
        session: 0,
        id: "fixture-probe-running".into(),
        result: Err("Old session".into()),
    });
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .live()
            .is_some()
    }));
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Err("Stats unavailable".into()),
    });
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .live()
            .is_none()
    }));
    h.checkpoint("metrics-unavailable");
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now() - crate::metrics::STALE_AFTER - Duration::from_secs(1),
            ..last_sample.clone()
        }),
    });
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .live()
            .is_none()
    }));
    h.checkpoint("metrics-stale");
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now(),
            ..last_sample
        }),
    });
    let generation = h.observe(|v| v.container_logs.generation);
    h.deliver(Event::Log {
        generation,
        data: b"2026-10-12T23:30:00.000000001Z \x1b[3".to_vec(),
    });
    h.deliver(Event::Log {
        generation,
        data: b"2mHEADLESS GPU FIXTURE READY\x1b[0m\n2026-10-12T23:30:00.000000002Z A real frame, without a desktop window.\n2026-10-12T23:30:00.000000003Z \x1b[32mbuilding 10%\r\x1b[2Kdone\x1b[0m\n"
            .to_vec(),
    });
    h.checkpoint("containers-and-logs");
    h.require_visible("cpu-history");
    h.require_visible("memory-history");
    h.require_visible("metrics-row-fixture-probe-running");
    h.require_visible("container-log-line-0");
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.lines[0].text.clone()),
        "HEADLESS GPU FIXTURE READY"
    );
    let probe_color = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-probe-1", 0x171d21)
    });
    h.require_visible("source-color-fixture-probe-running");
    h.require_visible("container-log-line-1");
    h.require_visible("container-log-line-2");
    assert!(h.observe(|v| v.container_logs.buffer.lines[2].text == "done"));
    h.hover("cpu-history", 0.35);
    h.checkpoint("cpu-chart-hover");
    let first_cpu_label = h.tooltip_label("cpu-hover-value");
    assert!(first_cpu_label.starts_with("CPU: ") && first_cpu_label.ends_with('%'));
    h.require_visible("cpu-hover-value");
    h.hover("cpu-history", 0.8);
    assert_ne!(
        h.tooltip_label("cpu-hover-value"),
        first_cpu_label,
        "Hover must follow the actual sample under the cursor"
    );
    h.checkpoint("cpu-chart-hover-next-sample");
    h.hover("memory-history", 0.5);
    h.checkpoint("memory-chart-hover");
    let memory_label = h.tooltip_label("memory-hover-value");
    assert!(
        h.observe(|v| v.metrics.containers["fixture-probe-running"]
            .samples
            .iter()
            .any(|sample| {
                sample.memory_used.is_some_and(|used| {
                    memory_label == format!("Memory: {}", crate::metrics::bytes(used))
                })
            })),
        "Memory hover must use a measured byte value, rather than the capacity percentage"
    );
    h.require_visible("memory-hover-value");
    h.hover("search", 0.5);
    // The reported usability case: a small working set against a large limit.
    for index in 0..20 {
        h.deliver(Event::Metrics {
            session: 1,
            id: "fixture-probe-running".into(),
            result: Ok(crate::metrics::Sample {
                collected: Instant::now() - Duration::from_secs((19 - index) * 2),
                cpu_percent: Some(0.12 + (index % 5) as f64 * 0.08),
                memory_used: Some(25 * 1024 * 1024 + 100 * 1024 + (index % 4) * 128 * 1024),
                memory_capacity: Some(2 * 1024 * 1024 * 1024),
                limits: Some(crate::metrics::Limits {
                    memory: Some(2 * 1024 * 1024 * 1024),
                    cpu_cores: Some(2.0),
                    cpuset: None,
                }),
                reset: index == 0,
            }),
        });
    }
    h.checkpoint("metrics-low-usage");
    h.require_visible("memory-history");
    h.hover("memory-history", 0.65);
    assert!(
        h.tooltip_label("memory-hover-value")
            .starts_with("Memory: 25.")
    );
    h.checkpoint("metrics-low-usage-hover");
    h.hover("search", 0.5);
    let expanded_logs =
        h.cx.debug_bounds(h.window.into(), "container-log-viewport")
            .unwrap()
            .unwrap();
    let retained_samples =
        h.observe(|v| v.metrics.containers["fixture-probe-running"].samples.len());
    let selected_before_collapse = h.observe(|v| v.containers.selected.clone());
    let logs_before_collapse = h.observe(|v| v.container_logs.buffer.text());
    h.click("toggle-metrics-graphs");
    h.checkpoint("metrics-collapsed-logs");
    for selector in [
        "cpu-history",
        "memory-history",
        "cpu-metrics",
        "memory-metrics",
    ] {
        assert!(
            h.cx.debug_bounds(h.window.into(), selector)
                .unwrap()
                .is_none()
        );
    }
    let collapsed_logs =
        h.cx.debug_bounds(h.window.into(), "container-log-viewport")
            .unwrap()
            .unwrap();
    assert!(
        collapsed_logs.size.height > expanded_logs.size.height + px(120.0),
        "Collapsing resources must give the recovered height to the logs"
    );
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.text()),
        logs_before_collapse
    );
    h.click_tab("container-tabs", 2);
    h.checkpoint("metrics-collapsed-inspect");
    assert!(
        h.cx.debug_bounds(h.window.into(), "cpu-history")
            .unwrap()
            .is_none()
    );
    h.click_tab("container-tabs", 0);
    h.click("toggle-metrics-graphs");
    h.checkpoint("metrics-expanded-again");
    assert!(
        h.cx.debug_bounds(h.window.into(), "cpu-history")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "memory-history")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        h.observe(|v| v.containers.selected.clone()),
        selected_before_collapse
    );
    assert_eq!(
        h.observe(|v| v.metrics.containers["fixture-probe-running"].samples.len()),
        retained_samples
    );
    assert!(
        !h.observe(|v| v.container_logs.buffer.text())
            .contains('\x1b')
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-stopped")
            .unwrap()
            .is_none()
    );

    // The settings UI owns theme changes; test assertions only read its state.
    let selected_before_settings = h.observe(|view| view.containers.selected.clone());
    h.click("settings");
    assert!(h.observe(|view| view.settings_open));
    h.checkpoint("settings-ferrydock");
    h.click("theme-light");
    let light_colors =
        h.cx.update_window(h.window.into(), |_, _, cx| crate::app::ui::palette(cx))
            .unwrap();
    assert!(
        light_colors.background > 0xd0d0d0,
        "Light theme must have a light application surface"
    );
    h.checkpoint("settings-light");
    h.click("settings");
    h.checkpoint("containers-light");
    h.click("toggle-compose-headless-fixture");
    h.checkpoint("compose-group-collapsed-light");
    h.click("toggle-compose-headless-fixture");
    h.require_visible("source-color-fixture-probe-running");
    h.require_visible("container-log-line-1");
    h.require_visible("container-log-line-0");
    assert_eq!(
        h.observe(|view| view.containers.selected.clone()),
        selected_before_settings
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(h.evidence.join("appearance.json")).unwrap())
            .unwrap();
    assert_eq!(saved["theme"], "light");
    h.click("settings");
    h.click("theme-dark");
    h.checkpoint("settings-dark");
    h.click("settings");
    h.checkpoint("containers-dark");
    h.key("ctrl-,");
    assert!(h.observe(|view| view.settings_open));
    h.click("theme-reset");
    h.checkpoint("settings-default-restored");
    h.click("settings");
    let restored =
        h.cx.update_window(h.window.into(), |_, _, cx| crate::app::ui::palette(cx))
            .unwrap();
    assert_eq!(restored.background, 0x171d21);
    assert_eq!(restored.text, TEXT);
    assert_eq!(
        h.observe(|view| view.containers.selected.clone()),
        selected_before_settings
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(h.evidence.join("appearance.json")).unwrap())
            .unwrap();
    assert_eq!(saved["theme"], "ferry-dock");
    h.click("container-more");
    h.checkpoint("container-action-menu");
    h.key("escape");
    h.click_button("row-fixture-probe-running", MouseButton::Right);
    h.checkpoint("container-context-menu");
    h.click_popup_item(2);
    assert!(
        h.observe(|view| view.tab == Tab::Inspect),
        "Context menu Inspect must use the same navigation as the tab"
    );
    h.click_tab("container-tabs", 0);

    h.click("all");
    assert!(!h.observe(|view| view.containers.running_only));
    h.checkpoint("containers-all");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-stopped")
            .unwrap()
            .is_some()
    );
    h.click("row-fixture-probe-stopped");
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-stopped".into(),
        result: Err("Late stopped sample".into()),
    });
    assert!(!h.observe(|v| v.metrics.containers.contains_key("fixture-probe-stopped")));
    h.checkpoint("metrics-stopped");
    h.scroll_containers(-120.0);
    h.click("row-fixture-standalone");
    let unlimited = h.observe(|v| {
        v.metrics.containers["fixture-standalone"]
            .samples
            .back()
            .unwrap()
            .clone()
    });
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-standalone".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now(),
            ..unlimited
        }),
    });
    h.checkpoint("metrics-no-container-limit");
    h.scroll_containers(120.0);
    h.click("row-fixture-probe-running");
    h.click("all");
    assert!(h.observe(|view| view.containers.running_only));

    h.click("search");
    assert!(
        h.cx.update_window(h.window.into(), |_, window, cx| {
            h.view.read(cx).containers.search_focus.is_focused(window)
        })
        .unwrap(),
        "Clicking the Input must focus its editor"
    );
    h.key("escape");
    h.key("ctrl-f");
    h.type_text("no-matching-container");
    assert_eq!(
        h.observe(|v| groups(
            &v.containers.items,
            v.containers.running_only,
            &v.containers.filter
        )
        .len()),
        0
    );
    h.checkpoint("empty-search");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_none()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_none()
    );

    h.key("ctrl-a");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "no-matching-container",
        "Select All must preserve the value until it is replaced"
    );
    h.type_text("dock");
    h.key("home");
    h.type_text("pre-");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "pre-dock",
        "Typing at Home must insert at the beginning"
    );
    h.key("shift-right");
    h.type_text("D");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "pre-Dock",
        "Typing must replace the selected character"
    );
    h.key("end");
    h.type_text("é");
    assert_eq!(h.observe(|v| v.containers.filter.clone()), "pre-Docké");
    h.key("backspace");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "pre-Dock",
        "Backspace must delete the complete Unicode character"
    );
    h.checkpoint("kit-input-editing");

    h.key("ctrl-a");
    h.type_text("headless-fixture");
    h.key("escape");
    h.key("down");
    assert_eq!(
        h.observe(|v| v.containers.filter.clone()),
        "headless-fixture"
    );
    assert_eq!(
        h.observe(|v| groups(
            &v.containers.items,
            v.containers.running_only,
            &v.containers.filter
        )
        .into_values()
        .flatten()
        .count()),
        1
    );
    h.checkpoint("filtered-fixture");
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-probe-running")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "row-fixture-standalone")
            .unwrap()
            .is_none()
    );

    h.click("follow");
    assert!(!h.observe(|v| v.container_logs.follow));
    h.checkpoint("logs-paused");
    let generation = h.observe(|v| v.container_logs.generation);
    h.deliver(Event::Log {
        generation,
        data: (0..200)
            .map(|index| {
                format!(
                    "SCROLLBAR ROW {index:03} {}\n",
                    "long log output ".repeat(20)
                )
            })
            .collect::<String>()
            .into_bytes(),
    });
    h.checkpoint("logs-scrollbar-top");
    let viewport =
        h.cx.debug_bounds(h.window.into(), "container-log-viewport")
            .unwrap()
            .expect("Rendered container log viewport");
    h.actions
        .push(json!({"input": "log-scrollbar-drag", "axis": "vertical"}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        let from = gpui::point(viewport.right() - px(6.0), viewport.origin.y + px(12.0));
        let to = gpui::point(from.x, viewport.bottom() - px(20.0));
        window.drag(from, to, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.settle_ui(300);
    assert!(
        h.observe(|v| v.container_logs.scroll.0.borrow().base_handle.offset().y < px(-1000.0)),
        "Dragging the vertical scrollbar must reveal later log rows"
    );
    h.actions
        .push(json!({"input": "log-scrollbar-drag", "axis": "horizontal"}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        let from = gpui::point(viewport.origin.x + px(12.0), viewport.bottom() - px(6.0));
        let to = gpui::point(viewport.right() - px(20.0), from.y);
        window.drag(from, to, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.settle_ui(300);
    assert!(
        h.observe(|v| v.container_logs.scroll.0.borrow().base_handle.offset().x < px(-100.0)),
        "Dragging the horizontal scrollbar must reveal long log records"
    );
    h.checkpoint("logs-scrollbar-dragged");
    h.click("follow");
    assert!(h.observe(|v| v.container_logs.follow));
    h.checkpoint("logs-scrollbar-following");
    assert_eq!(
        h.observe(|v| v.container_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );

    let initial_dock_height = h.dock_height();
    h.drag_dock(60.0);
    assert!(
        h.dock_height() < initial_dock_height - 20.0,
        "Kit divider must resize the dock through pointer input"
    );
    h.drag_dock(250.0);
    assert!(
        (h.dock_height() - 400.0).abs() < 2.0,
        "Dock minimum height must be enforced"
    );
    h.checkpoint("kit-dock-resized");
    h.drag_dock(h.dock_height() - initial_dock_height);
    assert!((h.dock_height() - initial_dock_height).abs() < 3.0);

    h.click_tab("container-tabs", 2);
    assert!(h.observe(|view| view.tab == Tab::Inspect));
    let generation = h.observe(|view| view.container_logs.generation);
    let inspection = serde_json::to_string_pretty(&json!({
        "Id": "fixture-probe-running",
        "Config": {"Image": "alpine:3.23", "Env": ["VISIBLE=fixture-only"]},
        "HostConfig": {"NetworkMode": "bridge"}
    }))
    .unwrap();
    h.deliver(Event::Inspection {
        generation,
        result: Ok(inspection.clone()),
    });
    assert_eq!(h.inspection_value(), inspection);
    h.checkpoint("kit-inspect-json");
    let view = h.view.clone();
    let body =
        h.cx.update_window(h.window.into(), |_, _, cx| {
            view.read(cx)
                .containers
                .inspection
                .read(cx)
                .text_bounds()
                .expect("Painted editor text bounds")
        })
        .unwrap();
    assert!(body.size.width > px(0.0) && body.size.height > px(0.0));
    h.key("ctrl-a");
    h.type_text("must-not-edit-inspection");
    assert_eq!(
        h.inspection_value(),
        inspection,
        "Inspect must remain read-only"
    );
    h.key("ctrl-f");
    h.type_text("Image");
    h.cx.read_entity(&h.view, |view, cx| {
        let editor = view.containers.inspection.read(cx);
        let search = editor.search_session();
        assert!(search.open);
        assert!(
            !search.replace_mode,
            "Read-only search must not expose replacement"
        );
        assert_eq!(search.query, "Image");
        assert_eq!(search.matcher.len(), 1);
    });
    h.checkpoint("kit-inspect-search");
    h.key("escape");
    h.click_tab("container-tabs", 0);
    assert!(h.observe(|view| view.tab == Tab::Logs));

    h.drain_commands();
    h.click("refresh");
    h.drain_commands();
    // Toolbar clicks preserve text focus; traverse the new global controls too.
    for id in ["quick-launcher", "quick-runs", "refresh"] {
        h.key("tab");
        h.cx.update_window(h.window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find(id).focused(),
                Some(true),
                "Toolbar button {id} must own focus for activation"
            );
        })
        .unwrap();
    }
    for key in ["enter", "space"] {
        h.key(key);
        let commands = h.drain_commands();
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::Refresh))
                .count(),
            1,
            "Refresh activation through {key} must fire once"
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::Terminal { .. })),
            "Button activation must not leak to the list shortcut"
        );
    }

    h.drain_commands();
    h.click("primary-action");
    assert!(h.drain_commands().iter().any(|command| matches!(command,
        Command::Action { id, action: Action::Stop } if id == "fixture-probe-running")));
    h.checkpoint("stop-requested");
    h.click("container-more");
    h.click_popup_item(8);
    assert!(
        h.drain_commands().is_empty(),
        "Disabled menu action must not repeat an in-flight operation"
    );
    h.key("escape");
    h.click("primary-action");
    assert!(
        h.drain_commands().is_empty(),
        "Disabled loading button must not repeat the action"
    );
    h.deliver(Event::ActionFinished {
        id: "fixture-probe-running".into(),
        result: Ok(()),
    });
    assert_eq!(h.notification_count(), 1);
    h.settle_ui(500);
    h.checkpoint("kit-action-notification");
    h.dismiss_notification();
    h.settle_ui(1000);
    assert_eq!(
        h.notification_count(),
        0,
        "Notification close control must dismiss feedback"
    );
    h.deliver(Event::ComposeSettingsError(
        "Fixture settings error remains visible".into(),
    ));
    h.settle_ui(500);
    h.cx.executor().advance_clock(Duration::from_secs(6));
    h.cx.run_until_parked();
    h.settle_ui(300);
    assert_eq!(
        h.notification_count(),
        1,
        "Error feedback must remain until manually dismissed"
    );
    h.checkpoint("kit-error-notification");
    h.dismiss_notification();
    h.settle_ui(1000);
    assert_eq!(h.notification_count(), 0);

    // Select the row before the list's Enter shortcut.
    h.click("row-fixture-probe-running");
    h.key("enter");
    assert!(h.observe(|v| v.tab == Tab::Terminal));
    let terminal_generation = h.observe(|v| v.terminal.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command,
        Command::Terminal { id, .. } if id == "fixture-probe-running")));
    h.deliver(Event::TerminalReady {
        generation: terminal_generation,
    });
    h.deliver(Event::TerminalData {
        generation: terminal_generation,
        data: b"\x1b[32mFerryDock headless GPU terminal\x1b[0m\r\n/ # ".to_vec(),
    });
    let terminal_sample = h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .samples
            .back()
            .unwrap()
            .clone()
    });
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(crate::metrics::Sample {
            collected: Instant::now(),
            ..terminal_sample.clone()
        }),
    });
    h.checkpoint("terminal-rendered");
    h.require_visible("cpu-metrics");
    h.require_visible("memory-metrics");
    verify_container_row_actions(&mut h);
    h.drain_commands();
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.within("shell-choice").click(1, cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    assert!(
        h.observe(|view| matches!(view.terminal.shell, crate::docker::Shell::Auto)),
        "A connected session must disable shell selection"
    );
    h.key("ctrl-,");
    assert!(
        h.observe(|view| view.settings_open),
        "Settings shortcut must work from a focused terminal"
    );
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("terminal-light");
    assert_eq!(
        h.observe(|view| view.terminal.generation),
        terminal_generation
    );
    assert!(
        !h.drain_commands()
            .iter()
            .any(|command| matches!(command, Command::Terminal { .. } | Command::CloseTerminal)),
        "Appearance changes must not recreate or close a live shell"
    );
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.type_text("echo verified");
    h.key("enter");
    let input: Vec<u8> = h
        .drain_commands()
        .into_iter()
        .filter_map(|command| match command {
            Command::Input { data, .. } => Some(data),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(input, b"echo verified\r");

    let log_generation = h.observe(|v| v.container_logs.generation);
    let terminal_text = h.cx.read_entity(&h.view, |v, cx| {
        v.terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .core
            .visible_text()
    });
    let inspection = h.inspection_value();
    let logs = h.observe(|v| v.container_logs.buffer.text());
    h.deliver(Event::Disconnected("Fixture Docker disconnected".into()));
    h.deliver(Event::Metrics {
        session: 1,
        id: "fixture-probe-running".into(),
        result: Ok(terminal_sample),
    });
    assert_eq!(h.observe(|v| v.metrics.session), None);
    assert!(h.observe(|v| {
        v.metrics.containers["fixture-probe-running"]
            .error
            .is_none()
    }));
    let log_status = h.observe(|v| v.container_logs.status.clone());
    let terminal_status = h.cx.read_entity(&h.view, |v, cx| {
        v.terminal.view.as_ref().unwrap().read(cx).status.clone()
    });
    for event in [
        Event::TerminalReady {
            generation: terminal_generation,
        },
        Event::TerminalData {
            generation: terminal_generation,
            data: b"STALE TERMINAL OUTPUT".to_vec(),
        },
        Event::TerminalEnded {
            generation: terminal_generation,
            error: Some("Stale shell error".into()),
        },
        Event::Log {
            generation: log_generation,
            data: b"STALE LOG OUTPUT\n".to_vec(),
        },
        Event::LogsEnded {
            generation: log_generation,
            error: Some("Stale log error".into()),
        },
        Event::Inspection {
            generation: log_generation,
            result: Ok("Stale inspection".into()),
        },
    ] {
        h.deliver(event);
        assert!(
            h.cx.read_entity(&h.view, |v, cx| !v
                .terminal
                .view
                .as_ref()
                .unwrap()
                .read(cx)
                .ready),
            "Disconnected shells must ignore stale ready events"
        );
    }
    assert_eq!(
        h.cx.read_entity(&h.view, |v, cx| v
            .terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .core
            .visible_text()),
        terminal_text
    );
    assert_eq!(
        h.cx.read_entity(&h.view, |v, cx| v
            .terminal
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .status
            .clone()),
        terminal_status
    );
    assert_eq!(h.observe(|v| v.container_logs.buffer.text()), logs);
    assert_eq!(h.observe(|v| v.container_logs.status.clone()), log_status);
    assert_eq!(h.inspection_value(), inspection);
    h.checkpoint("disconnected-stale-events-ignored");
    h.deliver(Event::Connected(Connection {
        name: "Fixture Docker".into(),
        endpoint: "fixture://docker".into(),
    }));
    h.deliver(Event::Snapshot(h.observe(|v| v.containers.items.clone())));
    h.drain_commands();

    h.deliver(Event::ComposeProjects(vec![ComposeProject {
        name: "headless-fixture".into(),
        config: None,
        worktree: Some("/fixture/project".into()),
        hint: ComposeConfig {
            project: "headless-fixture".into(),
            directory: "/fixture/project".into(),
            files: vec!["compose.yaml".into(), "compose.dev.yaml".into()],
            ..Default::default()
        },
    }]));
    h.checkpoint("compose-source-worktree");
    h.require_visible("source-headless-fixture");
    assert!(
        h.cx.debug_bounds(h.window.into(), "worktree-headless-fixture")
            .unwrap()
            .is_some()
    );
    h.hover("source-headless-fixture", 0.3);
    h.settle_ui(1200);
    h.checkpoint("compose-source-tooltip");
    h.click("project-headless-fixture");
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::ProjectLogs { project, .. } if project == "headless-fixture")));
    let generation = h.observe(|v| v.project_logs.generation);
    let mut members = h.observe(|v| v.containers.items.clone());
    let mut worker = container(
        "fixture-worker",
        "headless-fixture-worker-1",
        "running",
        Some("headless-fixture"),
    );
    worker.service = Some("worker".into());
    members.push(worker.clone());
    h.deliver(Event::Snapshot(members));
    for (id, service, name, text) in [
        (
            "fixture-probe-running",
            "probe",
            "headless-fixture-probe-1",
            "2026-10-12T23:30:00.000000001Z \x1b[32mHTTP ready\x1b[0m",
        ),
        (
            "fixture-worker",
            "worker",
            "headless-fixture-worker-1",
            "2026-10-12T23:30:00.000000002Z Queue ready",
        ),
    ] {
        h.deliver(Event::ProjectLog {
            generation,
            line: crate::project_logs::Line {
                container: id.into(),
                service: service.into(),
                name: name.into(),
                text: text.into(),
                error: false,
            },
        });
    }
    h.deliver(Event::ProjectLogsStatus {
        generation,
        status: "3 project containers · 2 live streams · ordered by Docker time".into(),
    });
    h.click("project-more");
    h.checkpoint("project-action-menu");
    h.key("escape");
    h.click("project-log-follow");
    assert!(!h.observe(|v| v.project_logs.follow));
    h.checkpoint("project-logs-all");
    let worker_color = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-worker-1", 0x101819)
    });
    assert_ne!(probe_color, worker_color);
    h.require_visible("project-log-line-0");
    h.require_visible("project-log-line-1");
    h.require_visible("source-color-fixture-probe-running");
    h.require_visible("source-color-fixture-worker");
    assert!(
        !h.observe(|v| v.project_logs.history.text(None))
            .contains('\x1b')
    );
    assert!(
        !h.observe(|v| v.project_logs.history.text(None))
            .contains(".00000000")
    );
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("project-logs-light");
    let light_background =
        h.cx.update_window(h.window.into(), |_, _, cx| crate::app::ui::palette(cx).logs)
            .unwrap();
    let light_probe = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-probe-1", light_background)
    });
    let light_worker = h.observe(|v| {
        v.containers
            .source_colors
            .color("headless-fixture-worker-1", light_background)
    });
    assert_ne!(light_probe, light_worker);
    h.require_visible("project-log-line-0");
    h.require_visible("project-log-line-1");
    h.require_visible("source-color-fixture-probe-running");
    h.require_visible("source-color-fixture-worker");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.click("project-service-filter");
    h.type_text("probe");
    h.key("enter");
    assert_eq!(
        h.observe(|v| v.project_logs.service.clone()).as_deref(),
        Some("probe")
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-0")
            .unwrap()
            .is_some()
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-1")
            .unwrap()
            .is_none()
    );
    h.click("project-log-copy");
    assert_eq!(
        h.notification_count(),
        1,
        "Copy feedback must use a Kit notification"
    );
    h.checkpoint("project-logs-filtered");
    h.require_visible("project-log-line-0");
    h.click_tab("project-tabs", 1);
    assert!(h.observe(|view| view.projects.tab == crate::app::projects::ProjectTab::Operations));
    h.click_tab("project-tabs", 0);
    assert!(h.observe(|view| view.projects.tab == crate::app::projects::ProjectTab::Logs));
    h.click("project-service-filter");
    h.type_text("All services");
    h.key("enter");
    h.click("project-log-follow");
    assert!(h.observe(|v| v.project_logs.follow));
    let mut members = h.observe(|v| v.containers.items.clone());
    members.retain(|container| container.id != "fixture-worker");
    worker.id = "fixture-worker-recreated".into();
    members.push(worker);
    h.deliver(Event::Snapshot(members));
    h.deliver(Event::ProjectLog {
        generation,
        line: crate::project_logs::Line {
            container: "fixture-worker-recreated".into(),
            service: "worker".into(),
            name: "headless-fixture-worker-1".into(),
            text: "2026-10-12T23:30:01.000000001Z Worker recreated and live".into(),
            error: false,
        },
    });
    assert!(h.observe(|v| {
        v.project_logs
            .history
            .text(None)
            .contains("Worker recreated and live")
    }));
    h.checkpoint("project-logs-recreated");
    h.require_visible("project-log-line-2");
    h.require_visible("source-color-fixture-worker-recreated");
    h.click("project-log-refresh");
    let generation = h.observe(|v| v.project_logs.generation);
    assert!(h.drain_commands().iter().any(|command| matches!(command, Command::ProjectLogs { generation: current, .. } if *current == generation)));
    let restarted = crate::project_logs::Line {
        container: "fixture-probe-running".into(),
        service: "probe".into(),
        name: "headless-fixture-probe-1".into(),
        text: "2026-10-12T23:31:00Z \x1b[32mProbe restarted and ready\x1b[0m".into(),
        error: false,
    };
    let lines = std::iter::once(restarted)
        .chain((0..80).map(|index| crate::project_logs::Line {
            container: "fixture-worker-recreated".into(),
            service: "worker".into(),
            name: "headless-fixture-worker-1".into(),
            text: format!("2026-10-12T23:30:00.{index:09}Z Older worker history {index}"),
            error: false,
        }))
        .collect();
    h.deliver(Event::ProjectLogBatch { generation, lines });
    h.checkpoint("project-logs-restart-ordered");
    assert_eq!(
        h.observe(|v| v.project_logs.history.lines.back().unwrap().text.clone()),
        "Probe restarted and ready"
    );
    assert_eq!(
        h.observe(|v| v.project_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-0")
            .unwrap()
            .is_none()
    );
    h.require_visible("project-log-line-80");
    h.click("project-service-filter");
    h.type_text("probe");
    h.key("enter");
    assert_eq!(
        h.observe(|v| v.project_logs.service.clone()).as_deref(),
        Some("probe")
    );
    h.checkpoint("project-logs-restart-filtered");
    h.require_visible("project-log-line-80");
    h.click("project-service-filter");
    h.type_text("All services");
    h.key("enter");
    h.click("row-fixture-probe-running");
    let commands = h.drain_commands();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::CloseProjectLogs))
    );
    assert!(commands.iter().any(
        |command| matches!(command, Command::Logs { id, .. } if id == "fixture-probe-running")
    ));
    h.key("ctrl-4");
    h.key("ctrl-1");
    assert!(h.observe(|v| v.projects.selected.is_none() && v.tab == Tab::Logs));
    let timestamp = "2026-10-13T05:00:00.000000001Z";
    h.deliver(Event::Log {
        generation: h.observe(|v| v.container_logs.generation),
        data: format!("{timestamp} \x1b[32mbuilding 10%\r\x1b[2Kdone\x1b[0m\n").into_bytes(),
    });
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.text()),
        "done",
        "Returning from project tabs must keep Docker timestamps hidden across ANSI progress updates"
    );
    h.checkpoint("container-logs-after-project-tab");
    h.key("ctrl-4");
    h.key("ctrl-1");
    h.drain_commands();
    // The worker publishes the complete initial history in one event. Its first
    // painted frame must already contain the tail, without an earlier top frame.
    h.deliver(Event::Log {
        generation: h.observe(|v| v.container_logs.generation),
        data: (0..500)
            .map(|index| format!("2026-10-13T05:00:00.{index:09}Z INITIAL HISTORY {index:03}\n"))
            .collect::<String>()
            .into_bytes(),
    });
    h.checkpoint("logs-open-at-bottom");
    assert_eq!(
        h.observe(|v| v.container_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "container-log-line-499")
            .unwrap()
            .is_some(),
        "The newest initial log record must appear in the first frame"
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "container-log-line-0")
            .unwrap()
            .is_none(),
        "Initial history must not paint from the top before reaching the tail"
    );
    h.key("ctrl-4");
    assert_eq!(
        h.observe(|v| v.projects.selected.clone()).as_deref(),
        Some("headless-fixture")
    );
    h.drain_commands();
    h.click("compose-start");
    h.click("compose-rebuild");
    assert!(
        h.drain_commands().is_empty(),
        "Unassociated project must not run"
    );
    h.click("compose-configure");
    h.assert_compose_focus(0);
    let initial_directory = h.compose_values()[0].clone();
    h.key("ctrl-a");
    assert_eq!(
        h.compose_values()[0],
        initial_directory,
        "Select All must not clear the directory"
    );
    h.type_text("/fixture/associated");
    h.key("home");
    h.type_text("/temporary");
    assert_eq!(h.compose_values()[0], "/temporary/fixture/associated");
    h.key("ctrl-z");
    assert_eq!(
        h.compose_values()[0],
        "/fixture/associated",
        "Undo must restore the directory"
    );
    h.key("end");
    h.type_text("é");
    h.key("backspace");
    assert_eq!(h.compose_values()[0], "/fixture/associated");
    h.key("tab");
    h.assert_compose_focus(1);
    h.key("ctrl-a");
    h.type_text("base.yaml");
    h.key("enter");
    h.type_text("override.yaml");
    h.key("home");
    h.key("shift-right");
    h.type_text("O");
    assert_eq!(
        h.compose_values()[1],
        "base.yaml\nOverride.yaml",
        "Textarea must replace selected text at the caret"
    );
    h.key("ctrl-z");
    assert_eq!(h.compose_values()[1], "base.yaml\noverride.yaml");
    h.key("shift-tab");
    h.assert_compose_focus(0);
    h.key("tab");
    h.assert_compose_focus(1);
    h.key("tab");
    h.assert_compose_focus(2);
    h.type_text(".env.dev");
    h.key("tab");
    h.assert_compose_focus(3);
    h.type_text("dev");
    h.key("tab");
    h.assert_compose_focus(0);
    h.key("shift-tab");
    h.assert_compose_focus(3);
    h.click("compose-field-1");
    h.assert_compose_focus(1);
    h.key("ctrl-a");
    assert_eq!(h.compose_values()[1], "base.yaml\noverride.yaml");
    h.checkpoint("compose-association");
    h.click("compose-save");
    let saving_values = h.compose_values();
    h.type_text("must-not-change-while-saving");
    h.key("escape");
    assert_eq!(
        h.compose_values(),
        saving_values,
        "Saving must disable input and cancellation"
    );
    let config = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::ConfigureCompose { config } => Some(config),
            _ => None,
        })
        .expect("Association command");
    assert_eq!(config.directory, PathBuf::from("/fixture/associated"));
    assert_eq!(
        config.files,
        vec![PathBuf::from("base.yaml"), PathBuf::from("override.yaml")]
    );
    assert_eq!(config.env_files, vec![PathBuf::from(".env.dev")]);
    assert_eq!(config.profiles, vec!["dev"]);
    h.deliver(Event::ComposeConfigured {
        project: config.project.clone(),
        result: Ok(config),
    });
    h.settle_ui(500);
    h.drain_commands();
    h.click("compose-configure");
    h.key("ctrl-a");
    h.type_text("/discarded-edit");
    h.key("escape");
    assert!(
        h.observe(|view| view.projects.form.is_none()),
        "Escape must cancel an editable form"
    );
    assert!(
        h.drain_commands().is_empty(),
        "Cancellation must not save changes"
    );
    h.click("compose-start");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action: compose::Action::Start,
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Compose Start request");
    h.click("compose-stop");
    h.click("compose-rebuild");
    assert!(
        h.drain_commands().is_empty(),
        "Conflicting controls must be disabled"
    );
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"\x1b[33mContainer headless-fixture-probe-1 Starting".to_vec(),
    });
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"\r\x1b[2K\x1b[32mContainer headless-fixture-probe-1 Started\x1b[0m\n".to_vec(),
    });
    h.checkpoint("compose-operation-live");
    h.require_visible("compose-log-line-0");
    assert!(h.observe(|v| {
        let output = v.projects.operations["headless-fixture"].output.text();
        !output.contains('\x1b') && !output.contains("Starting") && output.contains("Started")
    }));
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    assert!(h.observe(|v| {
        v.projects.operations["headless-fixture"]
            .result
            .as_ref()
            .unwrap()
            .is_ok()
    }));
    h.click("compose-stop");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                operation,
                action: compose::Action::Stop,
                ..
            } => Some(operation),
            _ => None,
        })
        .expect("Compose Stop request");
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Fixture Compose failure output retained\n".to_vec(),
    });
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Err("Fixture Compose operation failed".into()),
    });
    h.checkpoint("compose-operation-failed");
    assert!(h.observe(|v| {
        !v.project_busy("headless-fixture")
            && v.projects.operations["headless-fixture"]
                .output
                .text()
                .contains("failure output retained")
    }));
    h.click("compose-restart");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                operation,
                action: compose::Action::Restart,
                ..
            } => Some(operation),
            _ => None,
        })
        .expect("Compose Restart request");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    h.click("compose-rebuild");
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action: compose::Action::Rebuild,
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Compose Rebuild & restart request");
    assert!(h.observe(|v| v.projects.tab == crate::app::projects::ProjectTab::Operations));
    for control in [
        "compose-start",
        "compose-stop",
        "compose-restart",
        "compose-rebuild",
        "compose-configure",
    ] {
        h.click(control);
    }
    assert!(
        h.drain_commands().is_empty(),
        "Rebuild must block duplicate and conflicting actions"
    );
    h.key("ctrl-1");
    h.drain_commands();
    h.click("primary-action");
    assert!(
        h.drain_commands().is_empty(),
        "Container controls must be blocked during a project rebuild"
    );
    h.click("row-primary-fixture-probe-running");
    h.click("row-restart-fixture-probe-running");
    assert!(
        h.drain_commands().is_empty(),
        "Row controls must share the project operation guard"
    );
    h.key("ctrl-4");
    h.click_tab("project-tabs", 1);
    h.drain_commands();
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Building probe image\nContainer headless-fixture-probe-1 Recreated\n".to_vec(),
    });
    h.checkpoint("compose-rebuild-live");
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Err("Fixture build failed".into()),
    });
    h.key("ctrl-1");
    h.key("ctrl-4");
    h.click_tab("project-tabs", 1);
    assert!(
        h.observe(|v| {
            let operation = &v.projects.operations["headless-fixture"];
            !operation.running
                && operation.result.as_ref().unwrap().is_err()
                && operation.output.text().contains("Building probe image")
        }),
        "Build failure and output must survive leaving the project and returning"
    );
    h.checkpoint("compose-rebuild-failed");
    h.drain_commands();
    h.click("project-more");
    h.checkpoint("compose-rebuild-menu");
    h.click_popup_item(6);
    let operation = h
        .drain_commands()
        .into_iter()
        .find_map(|command| match command {
            Command::Compose {
                project,
                operation,
                action: compose::Action::Rebuild,
            } if project == "headless-fixture" => Some(operation),
            _ => None,
        })
        .expect("Project menu Rebuild & restart request");
    h.deliver(Event::ComposeOutput {
        project: "headless-fixture".into(),
        operation,
        data: b"Probe image built\nConfigured services recreated and started\n".to_vec(),
    });
    h.deliver(Event::ComposeFinished {
        project: "headless-fixture".into(),
        operation,
        result: Ok(()),
    });
    assert!(h.observe(|v| {
        let operation = &v.projects.operations["headless-fixture"];
        !v.project_busy("headless-fixture") && operation.result.as_ref().unwrap().is_ok()
    }));
    h.checkpoint("compose-rebuild-completed");

    verify_project_cleanup(&mut h);

    h.click_tab("project-tabs", 0);
    let generation = h.observe(|v| v.project_logs.generation);
    let ingestion_started = Instant::now();
    for chunk in (0..1500).collect::<Vec<_>>().chunks(64) {
        for index in chunk {
            h.events
                .try_send(Event::ProjectLog {
                    generation,
                    line: crate::project_logs::Line {
                        container: format!("stress-{}", index % 10),
                        service: format!("service-{}", index % 10),
                        name: format!("stress-service-{}-1", index % 10),
                        text: format!(
                            "2026-10-12T23:30:02.000000001Z busy-service log line {index:04} {}",
                            "application output ".repeat(8)
                        ),
                        error: false,
                    },
                })
                .unwrap();
        }
        h.cx.executor().advance_clock(Duration::from_millis(40));
        h.cx.run_until_parked();
    }
    let ingestion_seconds = ingestion_started.elapsed().as_secs_f64();
    h.checkpoint("project-logs-busy");
    let rows_rendered = h.observe(|v| v.project_logs.rows_rendered.get());
    assert!(
        rows_rendered < 80,
        "Log layout must be limited to visible rows, not all 1,500 retained lines"
    );
    assert!(
        h.observe(|v| v.project_logs.scroll.is_scrolled_to_end()) == Some(true),
        "Follow must jump directly to the current tail"
    );
    assert!(
        h.cx.debug_bounds(h.window.into(), "project-log-line-1499")
            .unwrap()
            .is_some()
    );
    let batched_started = Instant::now();
    for chunk in (0..10_000)
        .collect::<Vec<_>>()
        .chunks(crate::project_logs::MAX_BATCH_LINES)
    {
        h.events
            .try_send(Event::ProjectLogBatch {
                generation,
                lines: chunk
                    .iter()
                    .map(|index| crate::project_logs::Line {
                        container: format!("stress-{}", index % 10),
                        service: format!("service-{}", index % 10),
                        name: format!("stress-service-{}-1", index % 10),
                        text: format!(
                            "2026-10-12T23:30:03.000000001Z batched busy log line {index:04}"
                        ),
                        error: false,
                    })
                    .collect(),
            })
            .unwrap();
    }
    for _ in 0..20 {
        h.cx.executor().advance_clock(Duration::from_millis(40));
        h.cx.run_until_parked();
        if h.observe(|v| {
            v.project_logs
                .history
                .lines
                .back()
                .is_some_and(|line| line.text.contains("line 9999"))
        }) {
            break;
        }
    }
    let batched_ingestion_seconds = batched_started.elapsed().as_secs_f64();
    assert!(h.observe(|v| {
        v.project_logs.history.lines.len() == 1500
            && v.project_logs
                .history
                .lines
                .back()
                .unwrap()
                .text
                .contains("line 9999")
    }));
    h.checkpoint("project-logs-batched-busy");
    assert!(h.observe(|v| v.project_logs.rows_rendered.get()) < 80);
    assert_eq!(
        h.observe(|v| v.project_logs.scroll.is_scrolled_to_end()),
        Some(true)
    );
    h.click("project-log-follow");
    assert!(
        !h.observe(|v| v.project_logs.follow),
        "Controls must remain responsive after a log flood"
    );
    let performance = json!({"input_lines":1500,"retained_lines":h.observe(|v| v.project_logs.history.lines.len()),"rows_rendered_in_frame":rows_rendered,"ingestion_seconds":ingestion_seconds,
        "batched_input_lines":10000,"batched_ingestion_seconds":batched_ingestion_seconds,"batched_rows_rendered":h.observe(|v| v.project_logs.rows_rendered.get()),"follow_reached_tail":true,"controls_respond_after_flood":true});
    std::fs::write(
        h.evidence.join("log-performance.json"),
        serde_json::to_vec_pretty(&performance).unwrap(),
    )
    .unwrap();

    verify_images_workspace(&mut h);
    verify_volumes_workspace(&mut h);
    verify_container_row_widths(&mut h);

    let mut report = json!({
        "display": null, "wayland_display": null, "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820,
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "log_performance": performance,
        "checks": ["fixture discovery", "CPU and memory rows with configured limits", "native area chart layout with CPU-percent and MiB axes", "input-driven crosshair and tooltip sample values", "low memory values against a 2 GiB limit", "collecting unavailable stale stopped and no-limit resource summaries", "metrics session guards reject old and stopped samples", "resource summaries remain visible above the terminal", "Kit appearance Settings pointer theme switching and default restoration", "theme preference persistence in isolated file", "light theme log visibility", "settings preserve selected resource", "Kit container action menu opens and dismisses", "Kit right-click Inspect menu navigation", "Kit project action menu and disabled unassociated controls", "Kit Checkbox running filter and stopped rows", "Kit menu action blocks duplicate operation", "Kit RadioGroup shell choice disabled during live session", "theme switching preserves terminal generation and input focus", "Kit service Select search and keyboard confirmation", "search pointer focus", "keyboard search", "Kit Input select all and replacement", "Kit Input caret navigation and selection", "Kit Input Unicode backspace", "empty matches", "restored selection",
            "Kit Button Tab focus and Enter Space activation without shortcut leakage", "Kit Button disabled loading action guard",
            "Kit container and project TabBar pointer selection", "Kit Resizable pointer drag and minimum height",
            "Kit Inspect read-only typing guard", "Kit Inspect JSON editor visibility", "Kit Inspect search without replacement",
            "Kit success notification and visible close control", "Kit error notification persists beyond success timeout",
            "container ANSI text and split sequences", "matching container markers and distinct project source colors", "source marker visibility in Light theme", "project ANSI payload styles and plain copy without Docker timestamps", "Compose ANSI text and carriage-return progress replacement",
            "log follow pointer toggle", "stop command at Docker boundary", "terminal ANSI rendering", "terminal input routing",
            "disconnection ignores stale terminal, log and inspection events", "project tab return keeps Docker timestamps hidden",
            "Compose association keyboard form", "Kit Compose caret selection and Unicode editing", "Kit Compose undo",
            "Kit Compose pointer focus and Tab Shift-Tab wraparound", "Kit Compose disabled editing while saving", "Kit Compose Escape cancellation",
            "ordered files, environment files and profiles", "unassociated and conflicting actions blocked",
            "Compose Start Stop Restart Rebuild requests", "Rebuild toolbar and project menu requests", "Rebuild duplicate and conflicting project/container actions blocked", "Rebuild live output and success", "Rebuild failure output retained after leaving and returning", "streamed operation output and retained failure",
            "project log request without association", "combined service prefixes", "project service filter rendered rows",
            "project log follow pause and copy feedback", "recreated member log rendering", "return to container log routing",
            "Images sidebar and preserved container session", "Images tag and ID keyboard search", "Images virtualized 10000-row inventory", "Images stopped-container reference prevents removal", "Images review Cancel confirm duplicate guards", "Images removal success failure and refreshed references", "Images stale discovery and disconnected operation guards", "Images light-theme rendering"]
    });
    report["checks"].as_array_mut().unwrap().extend(
        [
            "Container row Start Stop Restart and resource menu requests target their own IDs",
            "Row pending and project-operation guards reject conflicting actions",
            "Other-container actions preserve selected logs and ready terminal session",
            "Row actions render in FerryDock and Light themes and at 820/1440 widths",
            "Compose source path and linked worktree tag",
            "Pointer-hover ordered Compose files and worktree tooltip",
            "Project menu cleanup review Cancel and Escape prevent execution",
            "Cleanup checkboxes send exact volume and image retention options",
            "Cleanup review invalidates on changed configuration and membership",
            "Cleanup failure remains visible; successful discovery removes project heading",
            "Compose and standalone chevrons collapse and expand rendered groups",
            "Folding preserves the detail session and survives inventory refresh",
            "Search reveals folded matches and restores folds when cleared",
            "Keyboard selection skips folded rows and handles all groups folded",
            "Bordered Compose groups render expanded and folded in FerryDock and Light themes",
            "Images Overview History Inspect tab clicks and keyboard shortcuts",
            "Images history commands UTC dates MiB sizes unavailable fields and copy feedback",
            "Images resizable history table shows aligned columns and a 53-command sample",
            "Images native divider dragging and metadata toggles preserve widths and selected rows",
            "Images history filtering context-copy horizontal wheel and first-last row navigation",
            "Images full-command dialog double-click Enter Close read-only typing and search",
            "Images history virtualizes 2000 entries and renders the Light theme",
            "Images inspection JSON read-only typing guard and built-in search",
            "Images independent detail errors refresh retries and empty history",
            "Images selected-ID generation closed-workspace and disconnect detail guards",
            "Images column-header clicks sort ascending and descending with rendered row order",
            "Images numeric size sorting keeps unavailable last",
            "Images sorting survives search and refresh without selection or session changes",
            "Images Ctrl-click toggles multiple highlighted rows and supports empty selection",
            "Images multi-selection survives sorting filtering and refreshed snapshots",
            "Images bulk review includes hidden selections and sends exact immutable IDs",
            "Images partial bulk results retain failed selections; full success clears selection",
            "Images bulk duplicate and stale-operation guards",
            "Volumes sidebar preserves container and terminal sessions",
            "Volumes name driver and Compose-project keyboard search",
            "Volumes stopped reference guards and container navigation",
            "Volumes Cancel Escape confirmation and exact-name removal requests",
            "Volumes pending duplicate success failure and refreshed-reference guards",
            "Volumes stale generation and disconnected operation guards",
            "Volumes virtualized 10000-row inventory and Light theme",
            "Volumes sortable Name Driver Age References headers with aligned age cells",
            "Volumes age sorts by creation instant and unavailable stays last in both directions",
            "Volumes Ctrl-click selection persists through sorting search and fresh inventory",
            "Volumes bulk review includes hidden and protected selections",
            "Volumes partial results retain failures and complete success clears selection",
            "Volumes bulk duplicate stale-operation and disconnect guards",
            "Volumes prune anonymous/all-unused review Cancel Escape Enter guard and unfiltered request",
            "Volumes prune pending duplicate and container/image conflict guards",
            "Volumes prune success failure zero-space unavailable-space and cache/selection reconciliation",
            "Volumes prune workspace and disconnected stale-operation guards",
        ]
        .map(Value::from),
    );
    h.finish(report);
    println!("Kit interaction evidence: {}", h.evidence.display());
}
