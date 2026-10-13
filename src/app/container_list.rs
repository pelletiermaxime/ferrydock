//! Virtualized resource list using Kit rows, state tags, and empty states.
use super::FerryDock;
use super::ui::{empty_state, palette};
use crate::model::{Container, GroupKey, groups};
use gpui::{prelude::*, *};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Sizable, empty::EmptyContent, list::ListItem, menu::ContextMenuExt, spinner::Spinner, tag::Tag,
};

enum ResourceRow {
    Heading(GroupKey, String),
    Container(Box<Container>),
}

impl FerryDock {
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
                        "{} running / {} containers · project controls",
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
            rows.push(ResourceRow::Heading(project, count_label));
            rows.extend(
                containers
                    .into_iter()
                    .map(|container| ResourceRow::Container(Box::new(container))),
            );
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
        uniform_list(
            "containers-scroll",
            rows.len().max(1),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| match rows.get(index) {
                        Some(ResourceRow::Heading(project, count_label)) => {
                            let selected = matches!(project, GroupKey::Compose(name)
                                if this.projects.selected.as_ref() == Some(name));
                            let compose_project = match project {
                                GroupKey::Compose(name) => Some(name.clone()),
                                GroupKey::Standalone => None,
                            };
                            let menu_project = compose_project.clone();
                            let header = ListItem::new(SharedString::from(format!(
                                "project-{}",
                                project.label()
                            )))
                            .selected(selected)
                            .accessibility_label(project.label().to_owned())
                            .disabled(compose_project.is_none())
                            .h(px(56.0))
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
                            .bg(rgb(if selected {
                                colors.selected
                            } else {
                                colors.background
                            }))
                            .text_size(px(12.0))
                            .when_some(compose_project, |header, name| {
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
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(project.label().to_owned()),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(colors.muted))
                                            .child(count_label.clone()),
                                    ),
                            );
                            if let Some(name) = menu_project {
                                header
                                    .context_menu(this.project_menu(name, cx))
                                    .into_any_element()
                            } else {
                                header.into_any_element()
                            }
                        }
                        Some(ResourceRow::Container(container)) => {
                            let selected = this.projects.selected.is_none()
                                && this.containers.selected.as_ref() == Some(&container.id);
                            let running = container.running();
                            let id = container.id.clone();
                            let busy = this
                                .containers
                                .busy
                                .get(&container.id)
                                .map(|action| format!("{} in progress…", action.label()));
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
                                .px_3()
                                .py_2()
                                .rounded_md()
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
                                                .w(px(160.0))
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
                                        .child(
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
                                        ),
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
        .p_3()
        .into_any_element()
    }
}
