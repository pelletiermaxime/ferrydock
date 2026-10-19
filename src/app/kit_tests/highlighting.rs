//! Exercise the real read-only JSON editor with representative document sizes.
use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let mut h = Harness::new(test_cx);
    h.deliver(Event::Connected(Connection {
        name: "Highlighting fixture".into(),
        endpoint: "fixture://no-docker".into(),
    }));
    h.click("0-1");
    let generation = h.observe(|v| v.images.generation);
    let id = format!("sha256:{}", "a".repeat(64));
    h.deliver(Event::Images {
        generation,
        result: Ok(vec![crate::images::Image {
            id: id.clone(),
            tags: vec!["ferrydock-highlighting-fixture:latest".into()],
            size: Some(1_048_576),
            created: 1_792_000_000,
            digests: vec![],
            containers: vec![].into(),
        }]),
    });
    h.click_tab("image-tabs", 2);
    let generation = h.observe(|v| v.images.details.generation);
    let mut measurements = Vec::new();
    for entries in [64, 1024, 8192] {
        let mut samples = Vec::new();
        let mut bytes = 0;
        for sample in 0..5 {
            let text = serde_json::to_string_pretty(&json!({
                "Id": id,
                "Probe": "FERRY_HIGHLIGHT_PROBE",
                "Config": {"Env": (0..entries).map(|index| format!("FIXTURE_{index}=value-{sample}-{}", "x".repeat(96))).collect::<Vec<_>>()},
                "Nested": {"Flag": true, "Count": 42, "Missing": null}
            })).unwrap();
            bytes = text.len();
            let started = Instant::now();
            h.deliver(Event::ImageInspection {
                id: id.clone(),
                generation,
                result: Ok(text.clone()),
            });
            // Advance the virtual clock through the highlighter's debounce;
            // run_until_parked drains its background work before observation.
            h.settle_ui(250);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            h.cx.read_entity(&h.view, |view, cx| {
                let editor = view.images.details.editor.read(cx);
                assert_eq!(editor.value(), text);
                assert!(
                    editor.display_map().is_fold_candidate(0),
                    "JSON root must remain foldable"
                );
            });
        }
        h.click_tab("image-tabs", 2);
        let original = h.cx.read_entity(&h.view, |view, cx| {
            view.images.details.editor.read(cx).value()
        });
        h.key("ctrl-a");
        h.type_text("must-not-edit-json");
        h.cx.read_entity(&h.view, |view, cx| {
            assert_eq!(view.images.details.editor.read(cx).value(), original)
        });
        h.key("ctrl-f");
        h.type_text("FERRY_HIGHLIGHT_PROBE");
        h.cx.read_entity(&h.view, |view, cx| {
            assert_eq!(
                view.images
                    .details
                    .editor
                    .read(cx)
                    .search_session()
                    .matcher
                    .len(),
                1
            )
        });
        h.key("escape");
        h.checkpoint(&format!(
            "highlighting-{entries}-entries-readonly-search-foldable"
        ));
        let mut sorted = samples.clone();
        sorted.sort_by(f64::total_cmp);
        measurements.push(
            json!({"entries": entries, "bytes": bytes, "load_and_settle_ms": samples,
            "median_ms": sorted[2], "minimum_ms": sorted[0], "maximum_ms": sorted[4]}),
        );
    }
    let report = json!({
        "engine": "tree-sitter",
        "measurement": "Fixture inspection delivery plus real editor layout and drained highlighting work; five virtual frames, no GPU or wall-clock debounce wait",
        "profile": if cfg!(debug_assertions) { "development" } else { "release" },
        "measurements": measurements,
        "checks": ["JSON root fold candidates", "read-only typing protection", "built-in search on all sizes"]
    });
    std::fs::write(
        h.evidence.join("highlighting.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    h.finish(report);
}
