use super::*;
use gpui_kit::{
    base::TestSupportExt,
    component::{
        dialog::Dialog,
        list::{List, ListDelegate, ListItem},
    },
};
#[derive(Clone)]
pub(super) enum Launch {
    Favorite(u64),
    Replica(u64, String),
}
#[derive(Clone)]
pub(super) struct Row {
    pub title: String,
    pub detail: String,
    pub search: String,
    pub action: Launch,
}
pub(super) struct Launcher {
    owner: WeakEntity<FerryDock>,
    rows: Vec<Row>,
    matches: Vec<usize>,
    selected: Option<IndexPath>,
}
impl ListDelegate for Launcher {
    type Item = ListItem;
    fn items_count(&self, _: usize, _: &App) -> usize {
        self.matches.len()
    }
    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_lowercase();
        self.matches = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| query.split_whitespace().all(|word| r.search.contains(word)))
            .map(|(i, _)| i)
            .collect();
        cx.notify();
        Task::ready(())
    }
    fn set_selected_index(
        &mut self,
        index: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = index;
    }
    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let row = self.rows.get(*self.matches.get(index.row)?)?;
        let colors = crate::app::ui::palette(cx);
        Some(
            ListItem::new(SharedString::from(format!("quick-result-{}", index.row)))
                .h(px(64.0))
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_ellipsis()
                                .child(row.title.clone()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(colors.muted))
                                .text_ellipsis()
                                .child(row.detail.clone()),
                        ),
                ),
        )
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_4()
            .text_color(rgb(crate::app::ui::palette(cx).muted))
            .child("No matching favorites. Add commands using Favorites on a container or project.")
    }
    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let Some(row) = self
            .selected
            .and_then(|i| self.matches.get(i.row))
            .and_then(|i| self.rows.get(*i))
            .cloned()
        else {
            return;
        };
        let _ = self.owner.update(cx, |this, cx| match row.action {
            Launch::Favorite(id) => this.choose_quick_favorite(id, window, cx),
            Launch::Replica(id, container) => this.start_quick_favorite(id, container, window, cx),
        });
    }
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
        let _ = self
            .owner
            .update(cx, |this, cx| this.close_quick_surface(window, cx));
    }
}
impl FerryDock {
    pub(in crate::app) fn open_quick_launcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.favorites.loaded {
            Self::notify(
                self.favorites
                    .error
                    .clone()
                    .unwrap_or_else(|| "Favorite commands are still loading".into()),
                self.favorites.error.is_some(),
                window,
                cx,
            );
            return;
        }
        let endpoint = self.connection.as_ref().map(|c| c.endpoint.as_str());
        let mut favorites = self
            .favorites
            .items
            .iter()
            .filter(|f| Some(f.endpoint.as_str()) == endpoint)
            .collect::<Vec<_>>();
        favorites.sort_by(|a, b| {
            b.last_used
                .cmp(&a.last_used)
                .then(b.use_count.cmp(&a.use_count))
                .then(b.pinned.cmp(&a.pinned))
                .then(a.label.cmp(&b.label))
                .then(a.id.cmp(&b.id))
        });
        let rows = favorites
            .into_iter()
            .map(|f| {
                let count = self
                    .containers
                    .items
                    .iter()
                    .filter(|c| c.running() && f.target.matches(c))
                    .count();
                let state = match count {
                    0 => "not running".into(),
                    1 => "ready".into(),
                    n => format!("{n} instances · choose one"),
                };
                Row {
                    title: format!(
                        "{}{} · {}",
                        if f.pinned { "★ " } else { "" },
                        f.label,
                        f.target.label()
                    ),
                    detail: format!("{} · {state}", f.command.replace('\n', " · ")),
                    search: format!("{} {} {}", f.label, f.target.label(), f.command)
                        .to_lowercase(),
                    action: Launch::Favorite(f.id),
                }
            })
            .collect();
        self.show_quick_launcher("Run a favorite".into(), rows, window, cx);
    }
    pub(super) fn show_quick_launcher(
        &mut self,
        title: String,
        rows: Vec<Row>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = self
            .favorites
            .quick
            .return_focus
            .clone()
            .or_else(|| window.focused(cx));
        if self.favorites.quick.launcher.is_some() || self.favorites.quick.console_open {
            window.close_dialog(cx);
        }
        self.favorites.quick.return_focus = focus;
        self.favorites.quick.title = title;
        self.favorites.quick.error = None;
        self.favorites.quick.console_open = false;
        let count = rows.len();
        let owner = cx.entity().downgrade();
        let list = cx.new(|cx| {
            ListState::new(
                Launcher {
                    owner: owner.clone(),
                    matches: (0..count).collect(),
                    rows,
                    selected: None,
                },
                window,
                cx,
            )
            .searchable(true)
        });
        list.update(cx, |s, cx| {
            if count > 0 {
                s.set_selected_index(Some(IndexPath::new(0)), window, cx);
            }
            s.focus(window, cx);
        });
        self.favorites.quick.launcher = Some(list.clone());
        window.open_dialog(cx, move |dialog, _, cx| {
            owner
                .update(cx, |this, cx| this.quick_launcher_dialog(dialog, cx))
                .unwrap_or_else(|_| Dialog::new(cx))
        });
        list.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }
    fn quick_launcher_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .capture_key_down(cx.listener(Self::quick_run_key));
        if let Some(error) = &self.favorites.quick.error {
            body = body.child(
                div()
                    .text_color(rgb(crate::app::ui::palette(cx).error))
                    .child(error.clone()),
            );
        }
        if let Some(list) = &self.favorites.quick.launcher {
            body = body.child(
                div()
                    .id("quick-launcher-list")
                    .test_support()
                    .h(px(330.0))
                    .child(
                        List::new(list).search_placeholder("Search commands, projects, services…"),
                    ),
            );
        }
        dialog
            .title(self.favorites.quick.title.clone())
            .width(px(760.0))
            .overlay_closable(true)
            .on_ok(|_, _, _| false)
            .on_close(cx.listener(|this, _, window, cx| this.close_quick_surface(window, cx)))
            .child(body)
            .footer(
                div()
                    .flex()
                    .justify_between()
                    .w_full()
                    .text_sm()
                    .child("↑ ↓ choose · Enter run · Esc close")
                    .child("Recent commands first · current Docker endpoint"),
            )
    }
    pub(in crate::app) fn pinned_project_favorites(
        &self,
        project: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut row = div().flex().gap_2().flex_shrink_0();
        for f in self
            .favorites
            .items
            .iter()
            .filter(|f| {
                f.pinned
                    && self
                        .connection
                        .as_ref()
                        .is_some_and(|c| c.endpoint == f.endpoint)
                    && matches!(&f.target,Target::Service{project:p,..}if p==project)
            })
            .take(3)
        {
            let id = f.id;
            row = row.child(
                Self::button(
                    SharedString::from(format!("quick-pin-{id}")),
                    format!("▶ {}", f.label),
                    false,
                    false,
                    cx,
                )
                .max_w(px(180.0))
                .overflow_hidden()
                .tooltip(format!("{} · {}", f.target.label(), f.command))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    // Keep the same target/error/replica handling as the global launcher.
                    this.open_quick_launcher(window, cx);
                    this.choose_quick_favorite(id, window, cx);
                })),
            );
        }
        row
    }
}
