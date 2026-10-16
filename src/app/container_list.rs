//! Virtualized resource list using Kit rows, state tags, and empty states.
use super::FerryDock;
use super::ui::{empty_state, palette};
use crate::docker::Action;
use crate::model::{Container, GroupKey, groups};
use gpui::{prelude::*, *};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    empty::EmptyContent,
    list::ListItem,
    menu::{ContextMenuExt, DropdownMenu},
    scroll::{Scrollbar, ScrollbarMode},
    spinner::Spinner,
    tag::Tag,
    tooltip::Tooltip,
};

enum ResourceRow {
    Heading(GroupKey, String, bool),
    Container(Box<Container>, bool),
}

impl super::containers::Containers {
    pub(super) fn group_expanded(&self, group: &GroupKey) -> bool {
        !self.filter.is_empty() || !self.collapsed_groups.contains(group)
    }
}

impl FerryDock {
    fn container_row_actions(
        &self,
        container: &Container,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = palette(cx);
        let unavailable = self.container_action_unavailable(container);
        let pending = self.containers.busy.get(&container.id);
        let primary = if container.running() {
            Action::Stop
        } else {
            Action::Start
        };
        div()
            .id(SharedString::from(format!("row-actions-{}", container.id)))
            .w(px(216.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_end()
            .gap_1()
            // Disabled buttons and menu triggers must not select the row either.
            .on_click(|_, _, cx| cx.stop_propagation())
            .when(container.running(), |view| {
                let id = container.id.clone();
                view.child(
                    Button::new(SharedString::from(format!("row-restart-{id}")))
                        .small()
                        .ghost()
                        .label("Restart")
                        .icon(IconName::RefreshCw)
                        .text_color(rgb(colors.muted))
                        .disabled(unavailable)
                        .loading(matches!(pending, Some(Action::Restart)))
                        .tooltip(format!("Restart {}", container.name))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.container_action(&id, Action::Restart, window, cx);
                        })),
                )
            })
            .child({
                let id = container.id.clone();
                Button::new(SharedString::from(format!("row-primary-{id}")))
                    .small()
                    .ghost()
                    .label(primary.label())
                    .icon(if container.running() {
                        gpui_kit::assets::IconName::Square
                    } else {
                        gpui_kit::assets::IconName::Play
                    })
                    .text_color(rgb(colors.muted))
                    .disabled(unavailable)
                    .loading(matches!(pending, Some(Action::Start | Action::Stop)))
                    .tooltip(format!("{} {}", primary.label(), container.name))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.container_action(&id, primary, window, cx);
                    }))
            })
            .child(
                Button::new(SharedString::from(format!("row-more-{}", container.id)))
                    .small()
                    .ghost()
                    .icon(IconName::Ellipsis)
                    .text_color(rgb(colors.muted))
                    .tooltip(format!("Actions for {}", container.name))
                    .dropdown_menu(self.container_menu(container.id.clone(), cx)),
            )
    }

    pub(super) fn resources(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let mut groups = groups(
            &self.containers.items,
            self.containers.running_only,
            &self.containers.filter,
        );
        for project in &self.projects.items {
            if self.containers.filter.is_empty()
                || project
                    .name
                    .to_lowercase()
                    .contains(&self.containers.filter.to_lowercase())
            {
                groups
                    .entry(GroupKey::Compose(project.name.clone()))
                    .or_default();
            }
        }
        let mut rows = Vec::new();
        for (project, containers) in groups {
            let count = containers.len();
            let count_label = match &project {
                GroupKey::Compose(name) => {
                    let members = self
                        .containers
                        .items
                        .iter()
                        .filter(|container| container.project.as_ref() == Some(name))
                        .collect::<Vec<_>>();
                    format!(
                        "{} running · {} containers",
                        members
                            .iter()
                            .filter(|container| container.running())
                            .count(),
                        members.len()
                    )
                }
                GroupKey::Standalone => {
                    format!("{count} container{}", if count == 1 { "" } else { "s" })
                }
            };
            let expanded = self.containers.group_expanded(&project);
            rows.push(ResourceRow::Heading(
                project,
                count_label,
                expanded && count > 0,
            ));
            if expanded {
                rows.extend(
                    containers
                        .into_iter()
                        .enumerate()
                        .map(|(index, container)| {
                            ResourceRow::Container(Box::new(container), index + 1 == count)
                        }),
                );
            }
        }
        if rows.is_empty() {
            let (title, description) = if self.connection.is_none() {
                (
                    "Waiting for Docker…",
                    "FerryDock will reconnect automatically when Docker is available.",
                )
            } else if !self.containers.filter.is_empty() {
                (
                    "No containers match your search",
                    "Clear the search to see your containers.",
                )
            } else if self.containers.running_only {
                (
                    "No running containers",
                    "Show all containers to see stopped services.",
                )
            } else {
                (
                    "No containers on this Docker context",
                    "Refresh after creating a container or choose another Docker context.",
                )
            };
            return empty_state(title, description)
                .content(
                    EmptyContent::new()
                        .when(self.connection.is_none(), |content| {
                            content.child(Spinner::new())
                        })
                        .when(!self.containers.filter.is_empty(), |content| {
                            content.child(
                                Self::button("clear-search", "Clear search", false, false, cx)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.containers.search_input.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                    })),
                            )
                        })
                        .when(
                            self.containers.running_only
                                && self.connection.is_some()
                                && self.containers.filter.is_empty(),
                            |content| {
                                content.child(
                                    Self::button(
                                        "show-all-containers",
                                        "Show all containers",
                                        false,
                                        false,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.containers.running_only = false;
                                            cx.notify();
                                        },
                                    )),
                                )
                            },
                        ),
                )
                .into_any_element();
        }
        let list = uniform_list(
            "containers-scroll",
            rows.len().max(1),
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                // Keep names and actions usable as the window gets narrower.
                let show_ports = window.viewport_size().width >= px(1280.0);
                let show_metrics = window.viewport_size().width >= px(1000.0);
                range
                    .map(|index| match rows.get(index) {
                        Some(ResourceRow::Heading(project, count_label, has_children)) => {
                            let selected = matches!(project, GroupKey::Compose(name)
                                if this.projects.selected.as_ref() == Some(name));
                            let compose_project = match project {
                                GroupKey::Compose(name) => Some(name.clone()),
                                GroupKey::Standalone => None,
                            };
                            let menu_project = compose_project.clone();
                            let source = compose_project.as_ref().and_then(|name| {
                                this.projects
                                    .items
                                    .iter()
                                    .find(|project| &project.name == name)
                            });
                            let source_label = if matches!(project, GroupKey::Compose(_)) {
                                source
                                    .map(|project| project.source_label())
                                    .unwrap_or_else(|| "Compose file path unavailable".into())
                            } else {
                                "Individual containers".into()
                            };
                            let source_tooltip = source.map(|project| project.source_tooltip());
                            let group = project.clone();
                            let expanded = this.containers.group_expanded(project);
                            let toggle_id = match project {
                                GroupKey::Compose(name) => format!("toggle-compose-{name}"),
                                GroupKey::Standalone => "toggle-standalone".into(),
                            };
                            let toggle_label = format!(
                                "{} {}",
                                if expanded { "Collapse" } else { "Expand" },
                                project.label()
                            );
                            let toggle = Button::new(SharedString::from(toggle_id.clone()))
                                .debug_selector(move || toggle_id.clone())
                                .small()
                                .ghost()
                                .icon(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .accessibility_label(toggle_label.clone())
                                .tooltip(if this.containers.filter.is_empty() {
                                    toggle_label
                                } else {
                                    "Search reveals matching containers; clear search to collapse"
                                        .into()
                                })
                                .disabled(!this.containers.filter.is_empty())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if !this.containers.collapsed_groups.remove(&group) {
                                        this.containers.collapsed_groups.insert(group.clone());
                                    }
                                    cx.notify();
                                }));
                            let header = ListItem::new(SharedString::from(format!(
                                "project-{}",
                                project.label()
                            )))
                            .selected(selected)
                            .accessibility_label(project.label().to_owned())
                            .h(px(52.0))
                            .w_full()
                            .debug_selector({
                                let label = project.label().to_owned();
                                move || format!("project-{label}")
                            })
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_2()
                            .rounded_md()
                            .when(*has_children, |header| header.rounded_b_none())
                            .border_1()
                            .border_color(rgb(colors.border))
                            .bg(rgb(if selected {
                                colors.selected
                            } else {
                                colors.panel
                            }))
                            .text_size(px(12.0))
                            .when_some(compose_project.clone(), |header, name| {
                                header.cursor_pointer().on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.select_project(name.clone(), window, cx)
                                    },
                                ))
                            })
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(toggle)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .text_ellipsis()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .child(project.label().to_owned()),
                                                    )
                                                    .when(
                                                        source.is_some_and(|project| {
                                                            project.worktree.is_some()
                                                        }),
                                                        |title| {
                                                            title.child(
                                                                div()
                                                                    .id(SharedString::from(
                                                                        format!(
                                                                            "worktree-{}",
                                                                            project.label()
                                                                        ),
                                                                    ))
                                                                    .test_support()
                                                                    .flex_shrink_0()
                                                                    .debug_selector({
                                                                        let name = project
                                                                            .label()
                                                                            .to_owned();
                                                                        move || {
                                                                            format!(
                                                                                "worktree-{name}"
                                                                            )
                                                                        }
                                                                    })
                                                                    .child(
                                                                        Tag::secondary()
                                                                            .small()
                                                                            .outline()
                                                                            .child("Git worktree"),
                                                                    ),
                                                            )
                                                        },
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "source-{}",
                                                        project.label()
                                                    )))
                                                    .test_support()
                                                    .debug_selector({
                                                        let name = project.label().to_owned();
                                                        move || format!("source-{name}")
                                                    })
                                                    .text_ellipsis()
                                                    .text_size(px(10.0))
                                                    .text_color(rgb(colors.muted))
                                                    .when_some(source_tooltip, |line, text| {
                                                        line.tooltip(move |window, cx| {
                                                            Tooltip::new(text.clone())
                                                                .build(window, cx)
                                                        })
                                                    })
                                                    .child(source_label),
                                            ),
                                    )
                                    .when_some(compose_project, |row, name| {
                                        row.child(this.pinned_project_favorites(&name, cx))
                                    })
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_color(rgb(colors.muted))
                                            .child(count_label.clone()),
                                    ),
                            );
                            let header = if let Some(name) = menu_project {
                                header
                                    .context_menu(this.project_menu(name, cx))
                                    .into_any_element()
                            } else {
                                header.into_any_element()
                            };
                            div().h(px(56.0)).pt_1().child(header).into_any_element()
                        }
                        Some(ResourceRow::Container(container, last)) => {
                            let selected = this.projects.selected.is_none()
                                && this.containers.selected.as_ref() == Some(&container.id);
                            let running = container.running();
                            let id = container.id.clone();
                            let busy = this.containers.busy.get(&container.id).map(|action| {
                                if show_metrics {
                                    format!("{} in progress…", action.label())
                                } else {
                                    format!("{}…", action.label())
                                }
                            });
                            ListItem::new(SharedString::from(format!("row-{}", container.id)))
                                .selected(selected)
                                .accessibility_label(format!(
                                    "{} · {}",
                                    container.name, container.state
                                ))
                                .debug_selector({
                                    let id = container.id.clone();
                                    move || format!("row-{id}")
                                })
                                .h(px(56.0))
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_3()
                                .pl(px(38.0))
                                .pr_3()
                                .py_2()
                                .rounded_none()
                                .border_l_1()
                                .border_r_1()
                                .border_color(rgb(colors.border))
                                .when(*last, |row| row.border_b_1().rounded_b_md())
                                .bg(rgb(if selected {
                                    colors.selected
                                } else {
                                    colors.background
                                }))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select(id.clone(), window, cx);
                                    this.focus.focus(window, cx);
                                }))
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "source-color-{}",
                                                    container.id
                                                )))
                                                .test_support()
                                                .debug_selector({
                                                    let id = container.id.clone();
                                                    move || format!("source-color-{id}")
                                                })
                                                .w(px(4.0))
                                                .h(px(28.0))
                                                .flex_shrink_0()
                                                .rounded_full()
                                                .bg(rgb(this
                                                    .containers
                                                    .source_colors
                                                    .color(&container.name, colors.logs))),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_size(px(13.0))
                                                        .text_ellipsis()
                                                        .child(container.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(colors.muted))
                                                        .text_ellipsis()
                                                        .child(container.image.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .w(px(if show_metrics { 160.0 } else { 112.0 }))
                                                .flex_shrink_0()
                                                .text_size(px(12.0))
                                                .text_color(rgb(if running {
                                                    colors.accent
                                                } else {
                                                    colors.muted
                                                }))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_2()
                                                        .when(busy.is_some(), |view| {
                                                            view.child(Spinner::new().small())
                                                        })
                                                        .child(
                                                            (if running {
                                                                Tag::success()
                                                            } else {
                                                                Tag::secondary()
                                                            })
                                                            .small()
                                                            .outline()
                                                            .child(busy.unwrap_or_else(|| {
                                                                container.state.clone()
                                                            })),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(colors.muted))
                                                        .text_ellipsis()
                                                        .child(container.status.clone()),
                                                ),
                                        )
                                        .when(show_metrics, |row| {
                                            row.child(this.metrics_row(container, cx))
                                        })
                                        .when(show_ports, |row| {
                                            row.child(
                                                div()
                                                    .w(px(140.0))
                                                    .text_size(px(12.0))
                                                    .text_color(rgb(colors.muted))
                                                    .text_ellipsis()
                                                    .child(if container.ports.is_empty() {
                                                        "—".into()
                                                    } else {
                                                        container.ports.clone()
                                                    }),
                                            )
                                        })
                                        .child(this.container_row_actions(container, cx)),
                                )
                                .context_menu(this.container_menu(container.id.clone(), cx))
                                .into_any_element()
                        }
                        None => div()
                            .h(px(56.0))
                            .p_3()
                            .text_color(rgb(colors.muted))
                            .child(if this.connection.is_none() {
                                "Waiting for Docker…"
                            } else if !this.containers.filter.is_empty() {
                                "No containers match your search"
                            } else if this.containers.running_only {
                                "No running containers · switch to All to see stopped containers"
                            } else {
                                "No containers on this Docker context"
                            })
                            .into_any_element(),
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .track_scroll(&self.containers.scroll)
        .flex_1()
        .min_h_0()
        .p_3();
        div()
            .id("containers-viewport")
            .test_support()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(list)
            .child(
                Scrollbar::vertical(&self.containers.scroll)
                    .id("containers-scrollbar")
                    .mode(ScrollbarMode::Always),
            )
            .into_any_element()
    }
}
