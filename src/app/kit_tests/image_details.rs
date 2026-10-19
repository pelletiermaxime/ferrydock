use super::*;

pub(super) fn verify_image_details(h: &mut Harness, inventory: &[crate::images::Image]) {
    use crate::app::image_details::ImageTab;
    use bollard::models::ImageHistoryResponseItem;
    let id = inventory[0].id.clone();
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Overview));
    assert!(
        !h.drain_commands()
            .iter()
            .any(|c| matches!(c, Command::ImageDetails { .. }))
    );
    h.click_tab("image-tabs", 1);
    let generation = h.observe(|v| v.images.details.generation);
    assert!(h.drain_commands().iter().any(|c| matches!(c, Command::ImageDetails { id: current, generation: active } if current == &id && *active == generation)));
    h.checkpoint("images-history-loading");
    let rows = vec![
        ImageHistoryResponseItem {
            id: id.clone(),
            created: 1_792_000_000,
            created_by: "CMD [\"/bin/sh\"]".into(),
            size: 0,
            comment: "buildkit.dockerfile.v0".into(),
            ..Default::default()
        },
        ImageHistoryResponseItem {
            id: "<missing>".into(),
            created: 1_791_900_000,
            created_by: format!(
                "RUN /bin/sh -c {}",
                "echo complete-build-command; ".repeat(20)
            ),
            size: 3 * 1_048_576,
            ..Default::default()
        },
        ImageHistoryResponseItem {
            id: "<missing>".into(),
            created: 0,
            size: -1,
            ..Default::default()
        },
    ];
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation,
        result: Ok(rows.clone()),
    });
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation,
        result: Err("Fixture inspection unavailable".into()),
    });
    h.checkpoint("images-history");
    assert_eq!(
        h.observe(|v| v
            .images
            .details
            .history
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .len()),
        3
    );
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().columns, [true; 3]);
        assert_eq!(table.delegate().active_columns(), vec![0, 1, 2, 3, 4]);
        assert_eq!(table.delegate().visible.len(), 3);
    });
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let command = window.find("history-cell-1-1").bounds();
        let size = window.find("history-cell-1-2").bounds();
        assert!(
            command.right() <= size.origin.x,
            "Long commands must stay inside their column"
        );
    })
    .unwrap();
    h.history_click_line(0, MouseButton::Left);
    h.actions.push(json!({"input": "double-click", "scope": "image-history-table", "selector": "resizable-handle", "column": 2}));
    h.cx.update_window(h.window.into(), |_, window, cx| {
        let mut table = window.within("image-history-table");
        let header = table.find(("col-header", 2_usize)).bounds();
        table.click_with_options(
            ("col-header", 2_usize),
            gpui_kit::test::ClickOptions::new()
                .with_count(2)
                .with_offset(gpui::point(
                    header.size.width - px(1.0),
                    header.size.height / 2.0,
                )),
            cx,
        );
    })
    .unwrap();
    h.cx.run_until_parked();
    h.checkpoint("images-history-column-fitted");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert!(table.col_width(2).unwrap() >= px(70.0));
        assert!(
            table.col_width(2).unwrap() < px(100.0),
            "Size column must shrink to its header/content"
        );
        assert_eq!(table.delegate().widths[2], table.col_width(2).unwrap());
        assert_eq!(
            table.selected_row(),
            Some(0),
            "Auto-fit must preserve the selected command"
        );
    });
    let width = h.cx.read_entity(&h.view, |view, cx| {
        view.images
            .details
            .history_view
            .table
            .read(cx)
            .delegate()
            .widths[1]
    });
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let header = window
            .within("image-history-table")
            .find(("col-header", 1_usize))
            .bounds();
        let from = gpui::point(header.right() - px(1.0), header.center().y);
        window.drag(from, from + gpui::point(px(200.0), px(0.0)), cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    h.checkpoint("images-history-table-resized");
    h.cx.read_entity(&h.view, |view, cx| {
        assert!(
            view.images
                .details
                .history_view
                .table
                .read(cx)
                .delegate()
                .widths[1]
                > width + px(40.0),
            "Pointer dragging must resize the command column"
        )
    });
    for column in 0..3 {
        h.click("image-history-columns");
        h.click_popup_item(column);
    }
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().columns, [false; 3]);
        assert_eq!(table.selected_row(), Some(0));
        assert!(table.delegate().widths[1] > width + px(40.0));
    });
    h.checkpoint("images-history-command-only");
    h.key("ctrl-f");
    h.type_text("complete-build-command");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(
            view.images
                .details
                .history_view
                .table
                .read(cx)
                .delegate()
                .visible,
            vec![1]
        )
    });
    h.checkpoint("images-history-search");
    h.key("escape");
    h.history_click_line(1, MouseButton::Right);
    h.checkpoint("images-history-context-menu");
    h.click_popup_item(0);
    h.cx.update_window(h.window.into(), |_, _, cx| {
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some(rows[1].created_by.clone())
        )
    })
    .unwrap();
    h.history_click_line(1, MouseButton::Left);
    h.key("ctrl-c");
    h.key("enter");
    assert!(h.observe(|v| v.images.details.history_view.command_open));
    h.checkpoint("images-history-full-command");
    h.key("ctrl-a");
    h.type_text("must-not-edit-command");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(
            view.images
                .details
                .history_view
                .command_editor
                .read(cx)
                .value(),
            rows[1].created_by
        )
    });
    h.key("ctrl-f");
    h.type_text("complete-build-command");
    h.cx.read_entity(&h.view, |view, cx| {
        let search = view
            .images
            .details
            .history_view
            .command_editor
            .read(cx)
            .search_session();
        assert!(search.open && !search.replace_mode);
        assert_eq!(search.matcher.len(), 20);
    });
    h.checkpoint("images-history-full-command-search");
    h.key("escape");
    h.click("history-command-close");
    assert!(!h.observe(|v| v.images.details.history_view.command_open));
    h.double_click("history-row-1");
    assert!(
        h.observe(|v| v.images.details.history_view.command_open),
        "Double click must open the full command"
    );
    h.click("history-command-close");
    h.key("ctrl-f");
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    for column in 0..3 {
        h.click("image-history-columns");
        h.click_popup_item(column);
    }
    h.checkpoint("images-history-columns");
    let left =
        h.cx.debug_bounds(h.window.into(), "history-header-command")
            .unwrap()
            .unwrap()
            .origin
            .x;
    h.history_scroll(-300.0, 0.0);
    h.checkpoint("images-history-horizontal-scroll");
    let moved =
        h.cx.debug_bounds(h.window.into(), "history-header-command")
            .unwrap()
            .unwrap()
            .origin
            .x;
    assert!(moved < left - px(30.0));
    h.history_scroll(10_000.0, 0.0);
    h.cx.update_window(h.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let header = window
            .within("image-history-table")
            .find(("col-header", 1_usize))
            .bounds();
        let from = gpui::point(header.right() - px(1.0), header.center().y);
        window.drag(from, from - gpui::point(px(200.0), px(0.0)), cx);
    })
    .unwrap();
    h.cx.run_until_parked();
    let realistic = (0..53)
        .map(|index| ImageHistoryResponseItem {
            created_by: match index % 3 {
                0 => "USER www-data".into(),
                1 => format!("RUN /bin/sh -c echo build-step-{index}"),
                _ => "WORKDIR /var/www/html".into(),
            },
            ..rows[index % rows.len()].clone()
        })
        .collect();
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation,
        result: Ok(realistic),
    });
    h.checkpoint("images-history-53-commands");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().visible.len(), 53);
        assert!(table.visible_range().rows().len() >= 5);
    });
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation,
        result: Ok(rows.clone()),
    });
    h.click_tab("image-tabs", 2);
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Inspect));
    assert!(
        h.drain_commands().is_empty(),
        "Tab changes reuse selected-image results"
    );
    h.checkpoint("images-inspect-failed");
    h.key("ctrl-f");
    h.type_text("ferry");
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation,
        result: Err("Fixture error while searching inventory".into()),
    });
    h.type_text("dock");
    assert_eq!(
        h.observe(|v| v.images.filter.clone()),
        "ferrydock",
        "An async inspection error must preserve inventory-search focus"
    );
    h.key("ctrl-a");
    h.key("backspace");
    h.key("escape");
    // Refresh retries both endpoints without changing the selected tab.
    h.key("ctrl-r");
    let inventory_generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: inventory_generation,
        result: Ok(inventory.to_vec()),
    });
    let active = h.observe(|v| v.images.details.generation);
    assert_ne!(active, generation);
    let inspection = serde_json::to_string_pretty(&json!({
        "Id": id, "Architecture": "amd64", "Config": { "Cmd": ["/bin/sh"], "Env": ["IMAGE=fixture-only"] },
        "RootFS": { "Type": "layers", "Layers": ["sha256:fixture-layer"] }
    })).unwrap();
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation: active,
        result: Ok(inspection.clone()),
    });
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(rows.clone()),
    });
    // Clicking the tab must focus the Kit editor, even after an async load.
    h.click_tab("image-tabs", 2);
    h.checkpoint("images-inspect-json");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(view.images.details.editor.read(cx).value(), inspection)
    });
    let view = h.view.clone();
    let body =
        h.cx.update_window(h.window.into(), |_, _, cx| {
            view.read(cx)
                .images
                .details
                .editor
                .read(cx)
                .text_bounds()
                .unwrap()
        })
        .unwrap();
    assert!(body.size.width > px(0.0) && body.size.height > px(0.0));
    h.key("ctrl-a");
    h.type_text("must-not-edit-image-inspection");
    h.cx.read_entity(&h.view, |view, cx| {
        assert_eq!(view.images.details.editor.read(cx).value(), inspection)
    });
    h.key("ctrl-f");
    h.type_text("Architecture");
    h.cx.read_entity(&h.view, |view, cx| {
        let search = view.images.details.editor.read(cx).search_session();
        assert!(search.open && !search.replace_mode);
        assert_eq!(search.query, "Architecture");
        assert_eq!(search.matcher.len(), 1);
    });
    h.checkpoint("images-inspect-search");
    h.key("escape");
    h.click_tab("image-tabs", 1);
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(vec![rows[2].clone()]),
    });
    h.checkpoint("images-history-unavailable-values");
    let many = (0..2_000)
        .map(|index| ImageHistoryResponseItem {
            created_by: format!("RUN echo history-{index}"),
            ..rows[0].clone()
        })
        .collect();
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(many),
    });
    h.checkpoint("images-history-large");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.delegate().visible.len(), 2_000);
        assert!(table.visible_range().rows().len() < 40);
    });
    h.history_click_line(0, MouseButton::Left);
    h.key("end");
    h.checkpoint("images-history-tail");
    h.cx.read_entity(&h.view, |view, cx| {
        let table = view.images.details.history_view.table.read(cx);
        assert_eq!(table.selected_row(), Some(1_999));
        assert_eq!(table.visible_range().rows().end, 2_000);
    });
    h.click("settings");
    h.click("theme-light");
    h.click("settings");
    h.checkpoint("images-history-light");
    h.click("settings");
    h.click("theme-reset");
    h.click("settings");
    h.click("image-row-bbbbbbbbbbbb");
    let second = h.observe(|v| v.images.details.generation);
    assert!(
        h.observe(|v| v.images.details.history.is_none() && v.images.details.inspection.is_none())
    );
    h.deliver(Event::ImageHistory {
        id: id.clone(),
        generation: active,
        result: Ok(rows.clone()),
    });
    h.deliver(Event::ImageInspection {
        id: id.clone(),
        generation: second,
        result: Ok(inspection.clone()),
    });
    assert!(
        h.observe(|v| v.images.details.history.is_none() && v.images.details.inspection.is_none()),
        "Old generation and wrong image ID must be ignored"
    );
    h.deliver(Event::ImageHistory {
        id: inventory[1].id.clone(),
        generation: second,
        result: Ok(vec![]),
    });
    h.checkpoint("images-history-empty");
    h.deliver(Event::ImageHistory {
        id: inventory[1].id.clone(),
        generation: second,
        result: Err("Fixture history unavailable".into()),
    });
    h.checkpoint("images-history-failed");
    h.click("0-0");
    h.deliver(Event::ImageInspection {
        id: inventory[1].id.clone(),
        generation: second,
        result: Ok(inspection),
    });
    assert!(
        h.observe(|v| v.images.details.inspection.is_none()),
        "Closed workspace rejects detail completions"
    );
    h.click("0-1");
    let inventory_generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: inventory_generation,
        result: Ok(inventory.to_vec()),
    });
    h.click("image-row-aaaaaaaaaaaa");
    h.key("ctrl-3");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Inspect));
    h.key("ctrl-2");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::History));
    h.key("ctrl-1");
    assert!(h.observe(|v| v.images.details.tab == ImageTab::Overview));
    h.click("0-0");
    h.click("0-1");
    let inventory_generation = h.observe(|v| v.images.generation);
    h.deliver(Event::Images {
        generation: inventory_generation,
        result: Ok(inventory.to_vec()),
    });
    h.drain_commands();
}
