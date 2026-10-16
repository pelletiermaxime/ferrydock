//! Combined project log state, filtering, and virtualized rendering.
use super::FerryDock;
use super::ui::palette;
use crate::docker::Command;
use gpui::{prelude::*, *};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    IndexPath, Sizable,
    scroll::{Scrollbar, ScrollbarMode},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    status_bar::StatusBar,
    toolbar::Toolbar,
};

pub(super) struct ProjectLogs {
    pub(super) generation: u64,
    pub(super) history: crate::project_logs::History,
    pub(super) status: String,
    pub(super) follow: bool,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) service: Option<String>,
    picker: Entity<SelectState<SearchableVec<SharedString>>>,
    service_options: Vec<SharedString>,
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) rows_rendered: std::cell::Cell<usize>,
}
impl ProjectLogs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FerryDock>) -> Self {
        let options = vec![SharedString::from("All services")];
        let picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(options.clone()),
                Some(IndexPath::default()),
                window,
                cx,
            )
            .searchable(true)
        });
        cx.subscribe(
            &picker,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                let SelectEvent::Confirm(value) = event;
                this.project_logs.service = value
                    .as_ref()
                    .filter(|value| value.as_ref() != "All services")
                    .map(|value| value.to_string());
                if this.project_logs.follow {
                    this.project_logs.scroll.scroll_to_bottom();
                }
                cx.notify();
            },
        )
        .detach();
        Self {
            generation: 0,
            history: Default::default(),
            status: "Select a Compose project to view its logs".into(),
            follow: true,
            scroll: UniformListScrollHandle::default(),
            service: None,
            picker,
            service_options: options,
            #[cfg(all(test, feature = "headless-tests"))]
            rows_rendered: std::cell::Cell::new(0),
        }
    }
}

impl FerryDock {
    pub(super) fn sync_service_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let services = self
            .containers
            .items
            .iter()
            .filter(|container| container.project.as_ref() == self.projects.selected.as_ref())
            .filter_map(|container| container.service.clone())
            .chain(self.project_logs.history.services().cloned())
            // Keep the active filter visible even after that service disappears.
            .chain(self.project_logs.service.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let options = std::iter::once(SharedString::from("All services"))
            .chain(services.into_iter().map(SharedString::from))
            .collect::<Vec<_>>();
        let selected = SharedString::from(
            self.project_logs
                .service
                .clone()
                .unwrap_or_else(|| "All services".into()),
        );
        let changed = options != self.project_logs.service_options;
        let selection_changed =
            self.project_logs.picker.read(cx).selected_value() != Some(&selected);
        if changed || selection_changed {
            self.project_logs.picker.update(cx, |picker, cx| {
                if changed {
                    picker.set_items(SearchableVec::new(options.clone()), window, cx);
                }
                picker.set_selected_value(&selected, window, cx);
            });
            self.project_logs.service_options = options;
        }
    }

    pub(super) fn begin_project_logs(&mut self, project: String) {
        self.project_logs.generation += 1;
        self.project_logs.history = Default::default();
        for marker in crate::lifecycle::summarize(&self.lifecycle_logs.markers)
            .iter()
            .filter(|marker| marker.project.as_deref() == Some(&project))
        {
            self.project_logs
                .history
                .push_annotation(marker.project_line(), marker.details.clone());
        }
        self.project_logs.status = "Connecting to project logs…".into();
        self.backend.send(Command::CloseLogs);
        self.backend.send(Command::ProjectLogs {
            project,
            generation: self.project_logs.generation,
        });
    }

    pub(super) fn project_logs_view(&self, _name: &str, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        #[cfg(all(test, feature = "headless-tests"))]
        self.project_logs.rows_rendered.set(0);
        let controls = Toolbar::new("project-log-toolbar")
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(colors.border))
            .child(
                Self::button(
                    "project-log-follow",
                    if self.project_logs.follow {
                        "Following"
                    } else {
                        "Follow paused"
                    },
                    self.project_logs.follow,
                    false,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_logs.follow = !this.project_logs.follow;
                    if this.project_logs.follow {
                        this.project_logs.scroll.scroll_to_bottom();
                    }
                    cx.notify();
                })),
            )
            .child(
                Self::button("project-log-copy", "Copy logs", false, false, cx).on_click(
                    cx.listener(|this, _, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            this.project_logs
                                .history
                                .text(this.project_logs.service.as_deref()),
                        ));
                        Self::notify("Retained project logs copied", false, window, cx);
                        cx.notify();
                    }),
                ),
            )
            .child(
                Self::button(
                    "project-log-refresh",
                    "Refresh logs",
                    false,
                    self.connection.is_none(),
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.connection.is_some()
                        && let Some(project) = this.projects.selected.clone()
                    {
                        this.begin_project_logs(project);
                        cx.notify();
                    }
                })),
            )
            .content(div().flex_1())
            .child(
                Select::new(&self.project_logs.picker)
                    .id("project-service-filter")
                    .small()
                    .w(px(210.0))
                    .accessibility_label("Filter project logs by service")
                    .search_placeholder("Search services…"),
            );
        let indices = self
            .project_logs
            .history
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                self.project_logs
                    .service
                    .as_ref()
                    .is_none_or(|service| line.service == *service)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let widest = indices
            .iter()
            .enumerate()
            .max_by_key(|(_, index)| {
                let line = &self.project_logs.history.lines[**index];
                line.text.len() + line.service.len() + line.name.len()
            })
            .map(|(index, _)| index);
        let lines = uniform_list(
            "project-log-scroll",
            indices.len().max(1),
            cx.processor(move |this, range: std::ops::Range<usize>, _, _| {
                #[cfg(all(test, feature = "headless-tests"))]
                this.project_logs
                    .rows_rendered
                    .set(this.project_logs.rows_rendered.get() + range.len());
                range
                    .map(|item| {
                        if let Some(index) = indices.get(item)
                            && let Some(line) = this.project_logs.history.lines.get(*index)
                        {
                            let index = *index;
                            div()
                                .h(px(18.0))
                                .whitespace_nowrap()
                                .text_color(rgb(if line.error {
                                    colors.error
                                } else {
                                    colors.text
                                }))
                                .id(SharedString::from(format!("project-log-line-{index}")))
                                .test_support()
                                .debug_selector(move || format!("project-log-line-{index}"))
                                .when(line.styled.annotation, |row| {
                                    super::lifecycle_logs::marker_row(row, colors)
                                })
                                .when_some(line.styled.details.clone(), |row, detail| {
                                    row.tooltip(move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(detail.clone())
                                            .build(window, cx)
                                    })
                                })
                                .child(line.render_source_on(
                                    this.containers.source_colors.color(&line.name, colors.logs),
                                    if line.error {
                                        colors.error
                                    } else {
                                        colors.text
                                    },
                                    colors.logs,
                                ))
                                .into_any_element()
                        } else {
                            div()
                                .h(px(18.0))
                                .text_color(rgb(colors.muted))
                                .child("Waiting for log output from this project's services…")
                                .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .with_width_from_item(widest)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(&self.project_logs.scroll)
        .flex_1()
        .min_h_0()
        .p_3()
        .font_family(super::ui::MONOSPACE_FONT)
        .text_size(px(12.0));
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(colors.logs))
            .child(controls)
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(lines)
                    .child(
                        Scrollbar::new(&self.project_logs.scroll)
                            .id("project-log-scrollbar")
                            .mode(ScrollbarMode::Always),
                    ),
            )
            .child(
                StatusBar::new()
                    .left(self.project_logs.status.clone())
                    .right(self.lifecycle_logs.status()),
            )
    }
}
