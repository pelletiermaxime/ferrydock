//! Image build history in a virtualized, resizable GPUI Kit table.
use super::{
    FerryDock,
    image_details::ImageTab,
    ui::{MONOSPACE_FONT, empty_state, palette},
};
use bollard::models::ImageHistoryResponseItem;
use gpui::{prelude::*, *};
use gpui_kit::{
    base::{
        TestSupportExt,
        actions::{SelectFirst, SelectLast},
    },
    component::{
        Sizable, WindowExt,
        dialog::Dialog,
        input::{Editor, EditorState, Input, InputEvent, InputState},
        menu::{DropdownMenu, PopupMenu, PopupMenuItem},
        table::{Column, DataTable, TableDelegate, TableEvent, TableSelection, TableState},
    },
};
use std::sync::Arc;

pub(super) struct ImageHistory {
    pub(super) table: Entity<TableState<HistoryTable>>,
    pub(super) search: Entity<InputState>,
    pub(super) command_editor: Entity<EditorState>,
    pub(super) command_open: bool,
}

pub(super) struct HistoryTable {
    pub(super) rows: Arc<[ImageHistoryResponseItem]>,
    pub(super) visible: Vec<usize>,
    pub(super) columns: [bool; 3],
    pub(super) widths: [Pixels; 5],
    query: String,
    owner: WeakEntity<FerryDock>,
}

impl HistoryTable {
    pub(super) fn active_columns(&self) -> Vec<usize> {
        (0..5)
            .filter(|column| *column < 2 || self.columns[*column - 2])
            .collect()
    }

    fn filter(&mut self) {
        let query = self.query.to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                row.created_by
                    .to_lowercase()
                    .contains(&query)
                    .then_some(index)
            })
            .collect();
    }

    fn text(&self, row: usize, column: usize) -> String {
        let Some(index) = self.visible.get(row) else {
            return String::new();
        };
        let row = &self.rows[*index];
        match column {
            0 => (index + 1).to_string(),
            1 => {
                if row.created_by.is_empty() {
                    "—".into()
                } else {
                    row.created_by.replace(['\r', '\n'], " ")
                }
            }
            2 => u64::try_from(row.size)
                .ok()
                .map(|bytes| format!("{:.2}", bytes as f64 / 1_048_576.0))
                .unwrap_or_else(|| "—".into()),
            3 => (row.created > 0)
                .then(|| chrono::DateTime::from_timestamp(row.created, 0))
                .flatten()
                .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".into()),
            4 => {
                if row.id.is_empty() || row.id == "<missing>" {
                    "—".into()
                } else {
                    row.id.clone()
                }
            }
            _ => String::new(),
        }
    }
}

impl TableDelegate for HistoryTable {
    fn columns_count(&self, _: &App) -> usize {
        self.active_columns().len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.visible.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        let index = self.active_columns()[index];
        let (key, name) = [
            ("number", "#"),
            ("command", "Command"),
            ("size", "Size (MiB)"),
            ("created", "Created (UTC)"),
            ("layer", "Layer ID"),
        ][index];
        Column::new(key, name)
            .width(self.widths[index])
            .min_width(px(if index == 0 { 32.0 } else { 70.0 }))
            .resizable(index != 0)
            .movable(false)
    }
    fn fit_column(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Option<Pixels> {
        let column = self.column(index, cx);
        let source_column = self.active_columns()[index];
        let measure = |text: SharedString, font: Font, font_size: Pixels| {
            window
                .text_system()
                .shape_line(
                    text.clone(),
                    font_size,
                    &[TextRun {
                        len: text.len(),
                        font,
                        color: rgb(palette(cx).text).into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
                .width
        };
        let mut width = measure(
            column.name,
            Font {
                family: "Inter".into(),
                ..Default::default()
            },
            window.rem_size() * 0.875,
        );
        let font = Font {
            family: MONOSPACE_FONT.into(),
            ..Default::default()
        };
        for row in 0..self.visible.len() {
            // A full instruction remains available in View command. Bound shaping
            // work for unusually long commands; Kit caps resized columns at 1200px.
            let text: String = self.text(row, source_column).chars().take(512).collect();
            width = width.max(measure(text.into(), font.clone(), px(12.0)));
            if width >= px(1200.0) {
                break;
            }
        }
        Some(width)
    }
    fn render_th(
        &mut self,
        index: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let column = self.column(index, cx);
        div()
            .id(SharedString::from(format!("history-header-{}", column.key)))
            .test_support()
            .size_full()
            .text_sm()
            .text_color(rgb(palette(cx).muted))
            .child(column.name)
    }
    fn render_tr(
        &mut self,
        row: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let owner = self.owner.clone();
        let index = self.visible[row];
        div()
            .id(SharedString::from(format!("history-row-{index}")))
            .on_click(move |event, window, cx| {
                if event.click_count() == 2 {
                    let _ =
                        owner.update(cx, |this, cx| this.view_history_command(index, window, cx));
                }
            })
    }

    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let column = self.active_columns()[column];
        let text = self.text(row, column);
        let colors = palette(cx);
        div()
            .id(SharedString::from(format!(
                "history-cell-{}-{column}",
                self.visible[row]
            )))
            .test_support()
            .w_full()
            .min_w_0()
            .text_ellipsis()
            .font_family(MONOSPACE_FONT)
            .text_size(px(12.0))
            .text_color(rgb(if text == "—" || column == 0 {
                colors.muted
            } else {
                colors.text
            }))
            .when(column == 2, |cell| cell.text_right())
            .child(text)
    }
    fn context_menu(
        &mut self,
        row: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let index = self.visible[row];
        let command = self.rows[index].created_by.clone();
        let owner = self.owner.clone();
        let id = self.rows[index].id.clone();
        menu.item(
            PopupMenuItem::new("Copy command")
                .disabled(command.is_empty())
                .on_click(move |_, window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                    FerryDock::notify("History command copied", false, window, cx);
                }),
        )
        .item(
            PopupMenuItem::new("View command…").on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.view_history_command(index, window, cx));
            }),
        )
        .item(
            PopupMenuItem::new("Copy layer ID")
                .disabled(id.is_empty() || id == "<missing>")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                }),
        )
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        empty_state("No commands match", "Clear or change the history search.")
    }
}

impl ImageHistory {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FerryDock>) -> Self {
        let owner = cx.entity().downgrade();
        let table = cx.new(|cx| {
            TableState::new(
                HistoryTable {
                    rows: Arc::from([]),
                    visible: Vec::new(),
                    columns: [true; 3],
                    widths: [px(38.0), px(420.0), px(100.0), px(155.0), px(225.0)],
                    query: String::new(),
                    owner,
                },
                window,
                cx,
            )
            .sortable(false)
            .col_movable(false)
            .col_selectable(false)
            .loop_selection(false)
        });
        cx.observe(&table, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&table, |_, table, event, cx| {
            if let TableEvent::ColumnWidthsChanged(widths) = event {
                table.update(cx, |table, _| {
                    let columns = table.delegate().active_columns();
                    for (column, width) in columns.into_iter().zip(widths) {
                        table.delegate_mut().widths[column] = *width;
                    }
                });
            }
        })
        .detach();
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search commands…  Ctrl+F"));
        cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.images
                    .details
                    .history_view
                    .table
                    .update(cx, |table, cx| {
                        let selected = table
                            .selected_row()
                            .and_then(|row| table.delegate().visible.get(row))
                            .copied();
                        table.delegate_mut().query = query;
                        table.delegate_mut().filter();
                        let selected = selected.and_then(|index| {
                            table
                                .delegate()
                                .visible
                                .iter()
                                .position(|row| *row == index)
                        });
                        table.set_selection(
                            selected.map_or(TableSelection::None, TableSelection::Row),
                            cx,
                        );
                        table.scroll_to_row(selected.unwrap_or(0), cx);
                        cx.notify();
                    });
                cx.notify();
            }
        })
        .detach();
        Self {
            table,
            search,
            command_editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .folding(false)
                    .indent_guides(false)
                    .soft_wrap(false)
            }),
            command_open: false,
        }
    }

    pub(super) fn set_rows(
        &self,
        rows: Arc<[ImageHistoryResponseItem]>,
        cx: &mut Context<FerryDock>,
    ) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = rows;
            table.delegate_mut().filter();
            table.set_selection(TableSelection::None, cx);
            table.scroll_to_row(0, cx);
            cx.notify();
        });
    }
}

impl FerryDock {
    fn toggle_history_column(&mut self, column: usize, cx: &mut Context<Self>) {
        self.images
            .details
            .history_view
            .table
            .update(cx, |table, cx| {
                let selected = table.selection();
                let columns = &mut table.delegate_mut().columns;
                columns[column] = !columns[column];
                table.refresh(cx);
                table.set_selection(selected, cx);
                cx.notify();
            });
        cx.notify();
    }
    fn history_columns_menu(
        &self,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.entity().downgrade();
        move |mut menu, _, cx| {
            let Some(view) = owner.upgrade() else {
                return menu;
            };
            let columns = view
                .read(cx)
                .images
                .details
                .history_view
                .table
                .read(cx)
                .delegate()
                .columns;
            for (index, label) in ["Size (MiB)", "Created (UTC)", "Layer ID"]
                .into_iter()
                .enumerate()
            {
                let owner = owner.clone();
                menu = menu.item(PopupMenuItem::new(label).checked(columns[index]).on_click(
                    move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.toggle_history_column(index, cx));
                    },
                ));
            }
            menu
        }
    }
    pub(super) fn view_history_command(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Ok(rows)) = &self.images.details.history else {
            return;
        };
        let Some(row) = rows.get(index) else {
            return;
        };
        self.images
            .details
            .history_view
            .command_editor
            .update(cx, |editor, cx| {
                editor.set_value(row.created_by.clone(), window, cx)
            });
        self.images.details.history_view.command_open = true;
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let Some(owner) = owner.upgrade() else {
                return dialog;
            };
            owner.update(cx, |this, cx| {
                this.history_command_dialog(dialog, index, cx)
            })
        });
        self.images
            .details
            .history_view
            .command_editor
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }
    fn history_command_dialog(
        &self,
        dialog: Dialog,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Dialog {
        dialog
            .title(format!("Command #{}", index + 1))
            .width(px(900.0))
            .on_close(cx.listener(|this, _, window, cx| {
                this.images.details.history_view.command_open = false;
                this.images
                    .details
                    .history_view
                    .table
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }))
            .child(
                div()
                    .id("history-command-viewer")
                    .test_support()
                    .h(px(300.0))
                    .child(
                        Editor::new(&self.images.details.history_view.command_editor)
                            .readonly(true)
                            .h_full(),
                    ),
            )
            .footer(
                Self::button("history-command-close", "Close", false, false, cx).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.images.details.history_view.command_open = false;
                        window.close_dialog(cx);
                        this.images
                            .details
                            .history_view
                            .table
                            .focus_handle(cx)
                            .focus(window, cx);
                        cx.notify();
                    }),
                ),
            )
    }
    pub(super) fn history_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let history = &self.images.details.history_view;
        if self.images.details.tab != ImageTab::History || history.command_open {
            return false;
        }
        let table_focused = history.table.focus_handle(cx).contains_focused(window, cx);
        let search_focused = history.search.focus_handle(cx).contains_focused(window, cx);
        if !table_focused && !search_focused {
            return false;
        }
        let key = &event.keystroke;
        match key.key.as_str() {
            "f" if key.modifiers.control => history.search.focus_handle(cx).focus(window, cx),
            "escape" if search_focused => history.table.focus_handle(cx).focus(window, cx),
            "c" if key.modifiers.control && table_focused => {
                let table = history.table.read(cx);
                if let Some(index) = table
                    .selected_row()
                    .and_then(|row| table.delegate().visible.get(row))
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        table.delegate().rows[*index].created_by.clone(),
                    ));
                    Self::notify("History command copied", false, window, cx);
                }
            }
            "enter" => {
                if search_focused {
                    history.table.focus_handle(cx).focus(window, cx);
                } else {
                    let index = history
                        .table
                        .read(cx)
                        .selected_row()
                        .and_then(|row| history.table.read(cx).delegate().visible.get(row))
                        .copied();
                    if let Some(index) = index {
                        self.view_history_command(index, window, cx);
                    }
                }
            }
            _ => return false,
        }
        cx.stop_propagation();
        cx.notify();
        true
    }
    fn history_select_first(
        &mut self,
        _: &SelectFirst,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_select_edge(false, window, cx);
    }
    fn history_select_last(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        self.history_select_edge(true, window, cx);
    }
    fn history_select_edge(&mut self, last: bool, window: &mut Window, cx: &mut Context<Self>) {
        let history = &self.images.details.history_view;
        if !history.table.focus_handle(cx).contains_focused(window, cx) {
            return;
        }
        // Kit's Home/End defaults navigate columns; history uses first/last rows.
        history.table.update(cx, |table, cx| {
            let count = table.delegate().visible.len();
            if count > 0 {
                table.set_selected_row(if last { count - 1 } else { 0 }, cx);
            }
            cx.notify();
        });
        cx.stop_propagation();
    }
    pub(super) fn compact_history(&self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let visible = self
            .images
            .details
            .history_view
            .table
            .read(cx)
            .delegate()
            .visible
            .len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .capture_action(cx.listener(Self::history_select_first))
            .capture_action(cx.listener(Self::history_select_last))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(colors.muted))
                            .child(if visible == count {
                                format!("{count} commands · newest first")
                            } else {
                                format!("{visible} / {count} commands · newest first")
                            }),
                    )
                    .child(
                        div()
                            .id("history-search")
                            .test_support()
                            .w(px(220.0))
                            .child(Input::new(&self.images.details.history_view.search).small()),
                    )
                    .child(
                        Self::button("image-history-columns", "Columns", false, false, cx)
                            .dropdown_menu(self.history_columns_menu(cx)),
                    ),
            )
            .child(
                div()
                    .id("image-history-table")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .child(
                        DataTable::new(&self.images.details.history_view.table)
                            .xsmall()
                            .bordered(false),
                    ),
            )
            .into_any_element()
    }
}
