//! Input/render proof for Docker lifecycle annotations, using worker events.
use super::*;

fn marker(
    action: &str,
    seconds: u32,
    id: &str,
    project: &str,
    service: &str,
) -> crate::lifecycle::Marker {
    crate::lifecycle::Marker::from_event(&serde_json::from_value(json!({
        "Type":"container", "Action":action, "time":1792152000 + seconds as i64,
        "Actor":{"ID":id,"Attributes":{"name":format!("{project}-{service}-1"),
            "com.docker.compose.project":project,"com.docker.compose.service":service,"exitCode":"0"}}
    })).unwrap()).unwrap()
}

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Fixture".into(),
        endpoint: "fixture://lifecycle".into(),
    }));
    h.deliver(Event::LifecycleSession(7));
    let members = vec![
        container("probe", "lifecycle-probe-1", "running", Some("lifecycle")),
        container("other", "other-probe-1", "running", Some("other")),
    ];
    h.deliver(Event::Snapshot(members.clone()));
    h.click("row-probe");
    let generation = h.observe(|v| v.container_logs.generation);
    let at = marker("stop", 1, "probe", "lifecycle", "probe").timestamp;
    h.deliver(Event::Log {
        generation,
        data: format!(
            "{} \x1b[32mBefore restart\n",
            (at - chrono::Duration::seconds(1)).to_rfc3339()
        )
        .into_bytes(),
    });
    h.deliver(Event::LifecycleStatus {
        session: 7,
        live: true,
    });
    for action in ["die", "stop", "start", "restart"] {
        h.deliver(Event::Lifecycle {
            session: 7,
            marker: marker(action, 1, "probe", "lifecycle", "probe"),
        });
    }
    h.deliver(Event::Lifecycle {
        session: 7,
        marker: marker("stop", 1, "other", "other", "probe"),
    });
    assert!(
        h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::Refresh))
    );
    h.deliver(Event::Snapshot(members.clone()));
    let generation = h.observe(|v| v.container_logs.generation);
    h.deliver(Event::Log {
        generation,
        data: format!(
            "{} Before restart\n{} After restart\n",
            (at - chrono::Duration::seconds(1)).to_rfc3339(),
            (at + chrono::Duration::seconds(1)).to_rfc3339()
        )
        .into_bytes(),
    });
    let text = h.observe(|v| v.container_logs.buffer.text());
    assert!(text.find("Before restart") < text.find("Container restarted"));
    assert!(text.find("Container restarted") < text.find("After restart"));
    assert_eq!(text.matches("Container stopped").count(), 0);
    assert_eq!(text.matches("Container restarted").count(), 1);
    h.checkpoint("lifecycle-container");
    h.click("copy-logs");
    h.click("refresh-logs");
    let generation = h.observe(|v| v.container_logs.generation);
    h.deliver(Event::Log {
        generation,
        data: format!(
            "{} After refresh\n",
            (at + chrono::Duration::seconds(2)).to_rfc3339()
        )
        .into_bytes(),
    });
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.text())
            .matches("Container restarted")
            .count(),
        1
    );
    h.checkpoint("lifecycle-container-refreshed");
    h.click("project-lifecycle");
    assert_eq!(
        h.observe(|v| v.project_logs.history.text(None))
            .matches("Container stopped")
            .count(),
        0
    );
    let generation = h.observe(|v| v.project_logs.generation);
    h.deliver(Event::ProjectLog {
        generation,
        line: crate::project_logs::Line {
            container: "probe".into(),
            service: "probe".into(),
            name: "lifecycle-probe-1".into(),
            text: format!(
                "{} \x1b[32mApplication ready",
                (at + chrono::Duration::seconds(2)).to_rfc3339()
            ),
            error: false,
        },
    });
    h.deliver(Event::Lifecycle {
        session: 7,
        marker: marker("pause", 4, "probe", "lifecycle", "probe"),
    });
    h.checkpoint("lifecycle-project");
    h.click("project-service-filter");
    h.type_text("probe");
    h.key("enter");
    assert!(
        h.observe(|v| v.project_logs.history.text(Some("probe")))
            .contains("Container paused")
    );
    h.click("project-log-refresh");
    assert_eq!(
        h.observe(|v| v.project_logs.history.text(None))
            .matches("Container paused")
            .count(),
        1
    );
    h.deliver(Event::LifecycleStatus {
        session: 7,
        live: false,
    });
    assert!(
        h.observe(|v| v.lifecycle_logs.status())
            .contains("gaps possible")
    );
    h.checkpoint("lifecycle-events-reconnecting");
    h.deliver(Event::LifecycleStatus {
        session: 7,
        live: true,
    });
    h.deliver(Event::Lifecycle {
        session: 7,
        marker: marker("pause", 4, "probe", "lifecycle", "probe"),
    });
    assert_eq!(
        h.observe(|v| v.project_logs.history.text(None))
            .matches("Container paused")
            .count(),
        1
    );
    h.checkpoint("lifecycle-events-recovered");
    let before = h.observe(|v| v.lifecycle_logs.markers.len());
    h.deliver(Event::Lifecycle {
        session: 6,
        marker: marker("destroy", 5, "probe", "lifecycle", "probe"),
    });
    assert_eq!(h.observe(|v| v.lifecycle_logs.markers.len()), before);
    h.click("row-other");
    assert!(
        !h.observe(|v| v.container_logs.buffer.text())
            .contains("Container restarted")
    );
    h.click("row-probe");
    assert!(
        h.observe(|v| v.container_logs.buffer.text())
            .contains("Container restarted")
    );
    h.checkpoint("lifecycle-return-to-container");
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("lifecycle-light");
    h.drag_dock(-200.0);
    // Reproduce the user's restart sequence: image-configured quit signal,
    // forced stop, then exit 137. Keep raw values in hover details only.
    for (action, seconds, key, value) in [
        ("kill", 10, "signal", "3"),
        ("kill", 20, "signal", "9"),
        ("die", 21, "exitCode", "137"),
        ("stop", 21, "exitCode", "137"),
        ("start", 21, "exitCode", "137"),
        ("restart", 21, "exitCode", "137"),
    ] {
        let e = serde_json::from_value(json!({"Type":"container","Action":action,"time":1792152000+seconds,
            "Actor":{"ID":"probe","Attributes":{"name":"lifecycle-probe-1","com.docker.compose.project":"lifecycle","com.docker.compose.service":"probe",key:value}}})).unwrap();
        h.deliver(Event::Lifecycle {
            session: 7,
            marker: crate::lifecycle::Marker::from_event(&e).unwrap(),
        });
    }
    let text = h.observe(|v| v.container_logs.buffer.text());
    assert!(!text.contains("Container shutdown requested"));
    assert!(!text.contains("Container force stop requested"));
    assert!(!text.contains("Container process ended"));
    assert_eq!(text.matches("Container restarted").count(), 2);
    assert!(!text.contains("signal 3") && !text.contains("exit code 137"));
    h.checkpoint("lifecycle-friendly-restart-light");
    let forced = h.observe(|v| {
        (0..v.container_logs.buffer.display_len())
            .rfind(|i| {
                v.container_logs
                    .buffer
                    .display_line(*i)
                    .unwrap()
                    .text
                    .contains("Container restarted")
            })
            .unwrap()
    });
    assert!(
        h.observe(|v| v
            .container_logs
            .buffer
            .display_line(forced)
            .unwrap()
            .details
            .clone()
            .unwrap())
            .contains("signal: 9")
    );
    assert!(
        h.observe(|v| v
            .container_logs
            .buffer
            .display_line(forced)
            .unwrap()
            .details
            .clone()
            .unwrap())
            .contains("6 Docker events")
    );
    h.hover(&format!("container-log-line-{forced}"), 0.3);
    h.settle_ui(1200);
    h.checkpoint("lifecycle-signal-details-light");
    h.click("project-lifecycle");
    let forced = h.observe(|v| {
        v.project_logs
            .history
            .lines
            .iter()
            .rposition(|line| line.text.contains("Container restarted"))
            .unwrap()
    });
    assert!(
        h.observe(|v| v.project_logs.history.lines[forced]
            .styled
            .details
            .clone()
            .unwrap())
            .contains("signal: 9")
    );
    h.hover(&format!("project-log-line-{forced}"), 0.3);
    h.settle_ui(1200);
    h.checkpoint("lifecycle-project-signal-details-light");
    // An already-open project view must remove only the intermediate Docker
    // annotations and preserve interleaved application output and ANSI state.
    let generation = h.observe(|v| v.project_logs.generation);
    for (second, body) in [
        (25, "\x1b[32mBefore next restart"),
        (35, "During restart"),
        (42, "After next restart\x1b[0m"),
    ] {
        h.deliver(Event::ProjectLog {
            generation,
            line: crate::project_logs::Line {
                container: "probe".into(),
                service: "probe".into(),
                name: "lifecycle-probe-1".into(),
                text: format!(
                    "{} {body}",
                    chrono::DateTime::from_timestamp(1792152000 + second, 0)
                        .unwrap()
                        .to_rfc3339()
                ),
                error: false,
            },
        });
    }
    for (action, second) in [
        ("kill", 30),
        ("kill", 40),
        ("die", 41),
        ("stop", 41),
        ("start", 41),
        ("restart", 41),
    ] {
        let e = serde_json::from_value(json!({"Type":"container","Action":action,"time":1792152000+second,
            "Actor":{"ID":"probe","Attributes":{"name":"lifecycle-probe-1","com.docker.compose.project":"lifecycle","com.docker.compose.service":"probe","signal":if second==30 {"3"}else{"9"},"exitCode":"137"}}})).unwrap();
        h.deliver(Event::Lifecycle {
            session: 7,
            marker: crate::lifecycle::Marker::from_event(&e).unwrap(),
        });
    }
    let text = h.observe(|v| v.project_logs.history.text(None));
    assert_eq!(text.matches("Container restarted").count(), 3);
    assert!(
        !text.contains("Container shutdown requested")
            && !text.contains("Container force stop requested")
    );
    for body in [
        "Before next restart",
        "During restart",
        "After next restart",
    ] {
        assert!(text.contains(body));
    }
    assert!(text.find("During restart") < text.rfind("Container restarted"));
    assert!(text.rfind("Container restarted") < text.find("After next restart"));
    h.checkpoint("lifecycle-grouped-project-light");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.checkpoint("lifecycle-grouped-project-dark");
    let summary = h.observe(|v| {
        v.project_logs
            .history
            .lines
            .iter()
            .rposition(|line| line.text.contains("Container restarted"))
            .unwrap()
    });
    h.require_visible(&format!("project-log-line-{summary}"));
    let application = h.observe(|v| {
        v.project_logs
            .history
            .lines
            .iter()
            .position(|line| line.text.contains("During restart"))
            .unwrap()
    });
    h.require_visible(&format!("project-log-line-{application}"));
    h.drag_dock(200.0);
    h.click("row-probe");
    h.checkpoint("lifecycle-grouped-container-dark");
    h.drag_dock(-200.0);
    // A separate stop remains two phases, even when it escalates to SIGKILL.
    for (action, second, signal) in [
        ("kill", 50, "3"),
        ("kill", 60, "9"),
        ("stop", 61, ""),
        ("die", 61, ""),
    ] {
        let e = serde_json::from_value(json!({"Type":"container","Action":action,"time":1792152000+second,
            "Actor":{"ID":"probe","Attributes":{"name":"lifecycle-probe-1","com.docker.compose.project":"lifecycle","com.docker.compose.service":"probe","signal":signal,"exitCode":"137"}}})).unwrap();
        h.deliver(Event::Lifecycle {
            session: 7,
            marker: crate::lifecycle::Marker::from_event(&e).unwrap(),
        });
        assert_eq!(
            h.observe(|v| v.container_logs.buffer.text())
                .matches("Container stop requested")
                .count(),
            1
        );
    }
    let text = h.observe(|v| v.container_logs.buffer.text());
    assert_eq!(text.matches("Container stopped").count(), 1);
    assert!(!text.contains("force stop requested") && !text.contains("Container process ended"));
    h.checkpoint("lifecycle-stop-container-dark");
    let stopped = h.observe(|v| {
        (0..v.container_logs.buffer.display_len())
            .rfind(|i| {
                v.container_logs
                    .buffer
                    .display_line(*i)
                    .unwrap()
                    .text
                    .contains("Container stopped")
            })
            .unwrap()
    });
    let details = h.observe(|v| {
        v.container_logs
            .buffer
            .display_line(stopped)
            .unwrap()
            .details
            .clone()
            .unwrap()
    });
    assert!(
        details.contains("signal: 3")
            && details.contains("signal: 9")
            && details.contains("exit code: 137")
    );
    h.hover(&format!("container-log-line-{stopped}"), 0.3);
    h.settle_ui(1200);
    h.checkpoint("lifecycle-stop-details-dark");
    h.deliver(Event::Lifecycle {
        session: 7,
        marker: marker("start", 73, "probe", "lifecycle", "probe"),
    });
    assert_eq!(
        h.observe(|v| v.container_logs.buffer.text())
            .matches("Container started")
            .count(),
        1
    );
    h.hover("copy-logs", 0.5);
    h.settle_ui(300);
    h.checkpoint("lifecycle-stop-then-start-container-dark");
    h.click("project-lifecycle");
    let text = h.observe(|v| v.project_logs.history.text(Some("probe")));
    assert_eq!(text.matches("Container stop requested").count(), 1);
    assert_eq!(text.matches("Container stopped").count(), 1);
    assert_eq!(text.matches("Container started").count(), 1);
    assert_eq!(text.matches("Container restarted").count(), 3);
    h.click("project-log-refresh");
    assert_eq!(
        h.observe(|v| v.project_logs.history.text(Some("probe")))
            .matches("Container stopped")
            .count(),
        1
    );
    h.checkpoint("lifecycle-stop-project-dark");
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("lifecycle-stop-project-light");
    let before = h.observe(|v| v.lifecycle_logs.markers.len());
    h.deliver(Event::Disconnected("fixture disconnected".into()));
    h.deliver(Event::Lifecycle {
        session: 7,
        marker: marker("destroy", 6, "probe", "lifecycle", "probe"),
    });
    assert_eq!(h.observe(|v| v.lifecycle_logs.markers.len()), before);
    h.deliver(Event::Connected(Connection {
        name: "Other endpoint".into(),
        endpoint: "fixture://other".into(),
    }));
    assert_eq!(h.observe(|v| v.lifecycle_logs.markers.len()), 0);
    assert!(
        !h.observe(|v| v.container_logs.buffer.text())
            .contains("[Docker]")
    );
    h.finish(json!({
        "checks":["external lifecycle markers in container and project logs", "timestamp order", "same-running snapshot reconnect", "refresh and navigation retention", "source isolation", "service filter", "copy control", "listener gap and recovery", "replay deduplication", "stale session rejection", "endpoint isolation", "Light theme", "six raw Docker events become one confirmed restart marker", "container and project full event hover details", "Kit Docker badge visibility in Light and Dark", "live project grouping preserves interleaved application output and ANSI state", "stop escalation produces one request and one completion", "separate later start remains distinct", "stop hover diagnostics and refresh retention"],
        "proof_boundary":"GPUI Kit input and layout with fixture worker events; Docker lifecycle and transport reconnection checked separately"
    }));
}
