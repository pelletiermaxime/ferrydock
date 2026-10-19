use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::app::image_details::ImageTab;
    use crate::image_analysis::{FileNode, Finding, Layer, Report};
    let started = Instant::now();
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Headless fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.click("0-1");
    let generation = h.observe(|v| v.images.generation);
    let id = format!("sha256:{}", "a".repeat(64));
    let inventory = vec![crate::images::Image {
        id: id.clone(),
        tags: vec!["ferrydock-analysis-demo:latest".into()],
        size: Some(12 * 1_048_576),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 3600,
        digests: vec![],
        containers: vec![].into(),
    }];
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory.clone()),
    });
    h.drain_commands();
    h.click_tab("image-tabs", 3);
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Analysis));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::AnalyzeImage { .. })),
        "Analysis is explicitly requested"
    );
    h.checkpoint("image-analysis-idle");
    h.click("image-analyze");
    let request = h.observe(|v| v.images.details.analysis.generation);
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::AnalyzeImage { id: current, generation } if current == &id && *generation == request)));
    h.checkpoint("image-analysis-loading");
    h.click("image-analysis-cancel");
    assert!(
        h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::CloseImageAnalysis))
    );
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: request,
        result: Err("stale cancelled result".into()),
    });
    assert!(h.observe(|v| v.images.details.analysis.loading));
    let report = Arc::new(Report {
        unpacked_bytes: 12 * 1_048_576,
        wasted_bytes: 4 * 1_048_576,
        efficiency: Some(66),
        layers: vec![
            Layer {
                command: "COPY application /app".into(),
                unpacked_bytes: 8 * 1_048_576,
                files: vec![FileNode {
                    name: "app".into(),
                    bytes: 8 * 1_048_576,
                    change: "Added".into(),
                    children: vec![
                        FileNode {
                            name: "app-server".into(),
                            bytes: 4 * 1_048_576,
                            change: "Added".into(),
                            ..Default::default()
                        },
                        FileNode {
                            name: "build-cache".into(),
                            bytes: 4 * 1_048_576,
                            change: "Added".into(),
                            children: vec![FileNode {
                                name: "compiler.tmp".into(),
                                bytes: 4 * 1_048_576,
                                change: "Added".into(),
                                ..Default::default()
                            }],
                        },
                    ],
                }],
                ..Default::default()
            },
            Layer {
                command: "RUN rm -rf /app/build-cache".into(),
                unpacked_bytes: 1_048_576,
                files: vec![FileNode {
                    name: "app".into(),
                    change: "Modified".into(),
                    children: vec![FileNode {
                        name: "build-cache".into(),
                        bytes: 4 * 1_048_576,
                        change: "Removed".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            Layer {
                command: "CMD [\"/app/app-server\"]".into(),
                metadata_only: true,
                ..Default::default()
            },
        ],
        findings: vec![Finding {
            title: "Reclaim wasted space".into(),
            detail: "A later layer removes a build cache retained by an earlier layer.".into(),
            hint: "Remove temporary files in the same RUN instruction that creates them.".into(),
            heuristic: true,
        }],
        ..Default::default()
    });
    h.deliver(Event::ImageAnalysis {
        id: "wrong-image".into(),
        generation: active,
        result: Ok(report.clone()),
    });
    assert!(h.observe(|v| v.images.details.analysis.loading));
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(report.clone()),
    });
    assert!(!h.observe(|v| v.images.details.analysis.loading));
    h.checkpoint("image-analysis-files");
    h.actions
        .push(json!({"input": "sidebar-drag", "delta": 60}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        let from = window
            .within("analysis-split")
            .find(("resizable-handle", 0_usize))
            .bounds()
            .center();
        window.drag(from, from + gpui::point(px(60.0), px(0.0)), cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.checkpoint("image-analysis-sidebar-dragged");
    let sidebar_width =
        h.cx.debug_bounds(h.window.into(), "analysis-layer-pane")
            .unwrap()
            .unwrap()
            .size
            .width;
    assert!(
        sidebar_width > px(300.0),
        "The fixed sidebar must remain manually resizable; actual width: {sidebar_width:?}"
    );
    h.actions
        .push(json!({"input": "window-resize", "width": 1440, "height": 820}));
    h.cx.simulate_window_resize(h.window.into(), size(px(1440.0), px(820.0)));
    h.cx.run_until_parked();
    h.checkpoint("image-analysis-fixed-sidebar-wide");
    assert_eq!(
        h.cx.debug_bounds(h.window.into(), "analysis-layer-pane")
            .unwrap()
            .unwrap()
            .size
            .width,
        sidebar_width,
        "Resizing the window must preserve the layer sidebar width"
    );
    h.cx.simulate_window_resize(h.window.into(), size(px(1180.0), px(820.0)));
    h.cx.run_until_parked();
    h.click("analysis-file-2"); // Expand build-cache via Kit Tree input.
    h.checkpoint("image-analysis-expanded-folder");
    assert!(
        h.cx.debug_bounds(h.window.into(), "analysis-file-3")
            .unwrap()
            .is_some(),
        "Expanded folder must expose its nested file"
    );
    h.click("analysis-layer-1");
    assert_eq!(h.observe(|v| v.images.details.analysis.layer), 1);
    h.checkpoint("image-analysis-deleted-files");
    h.click_tab("analysis-content-tabs", 1);
    h.checkpoint("image-analysis-findings");
    h.click_tab("analysis-content-tabs", 0);
    h.click("analysis-layer-2");
    h.checkpoint("image-analysis-metadata-only");
    h.key("ctrl-1");
    h.key("ctrl-4");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Analysis));
    assert!(h.observe(|v| v.images.details.analysis.result.is_some()));
    h.click("image-analyze");
    let empty_request = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: empty_request,
        result: Ok(Arc::new(Report {
            layers: vec![Layer {
                command: "LABEL empty=true".into(),
                metadata_only: true,
                ..Default::default()
            }],
            ..Default::default()
        })),
    });
    h.checkpoint("image-analysis-empty-efficiency");
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Err("Fixture export failed: image disappeared".into()),
    });
    h.checkpoint("image-analysis-failed");
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    let mut large = (*report).clone();
    large.layers = (0..2_000)
        .map(|index| Layer {
            command: format!("COPY fixture-{index} /app"),
            files: (0..10_000)
                .map(|i| FileNode {
                    name: format!("file-{i}"),
                    bytes: 1024,
                    change: "Added".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .take(1)
        .chain((1..2_000).map(|index| Layer {
            command: format!("LABEL fixture-layer={index}"),
            metadata_only: true,
            ..Default::default()
        }))
        .collect();
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(Arc::new(large)),
    });
    h.checkpoint("image-analysis-large");
    assert!(
        h.observe(|v| v.images.details.analysis.rows_rendered.get()) < 12,
        "Layer list must be virtualized"
    );
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    // Workspace navigation invalidates detail results; deliver a fresh inventory.
    let generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation,
        result: Ok(inventory),
    });
    h.click_tab("image-tabs", 3);
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(report.clone()),
    });
    h.checkpoint("image-analysis-light");
    h.click("image-analyze");
    let active = h.observe(|v| v.images.details.analysis.generation);
    h.key("ctrl-r");
    h.deliver(Event::ImageAnalysis {
        id: id.clone(),
        generation: active,
        result: Ok(report.clone()),
    });
    assert!(
        h.observe(|v| v.images.details.analysis.result.is_none()),
        "Refresh rejects stale analysis"
    );
    h.deliver(Event::Disconnected("Fixture disconnect".into()));
    h.deliver(Event::ImageAnalysis {
        id,
        generation: active,
        result: Ok(report),
    });
    assert!(h.observe(|v| v.images.details.analysis.result.is_none()));
    h.checkpoint("image-analysis-disconnected");
    let report = json!({ "docker_worker_started": false,
        "logical_width": 1180, "logical_height": 820,
        "render_and_flow_seconds": started.elapsed().as_secs_f64(),
        "checks": ["explicit Analyze request", "Cancel rejects old generation after retry", "wrong image rejected", "native Kit layer selection and tree expansion", "Files and Findings tabs", "metadata-only state", "zero-payload efficiency unavailable", "Ctrl+4 navigation and retained report", "failure retry", "2000 layers and 10000 files", "Light theme", "refresh and disconnect guards"] });
    h.finish(report);
}
