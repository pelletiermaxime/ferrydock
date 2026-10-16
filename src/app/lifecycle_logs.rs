//! Session-scoped lifecycle markers shared by individual and project log views.
use std::collections::VecDeque;

use super::FerryDock;
use crate::{
    ansi::Buffer,
    docker::Event,
    lifecycle::{MAX_MARKERS, Marker, summarize},
};

#[derive(Default)]
pub(super) struct LifecycleLogs {
    pub session: Option<u64>,
    endpoint: Option<String>,
    pub markers: VecDeque<Marker>,
    live: bool,
    gap: bool,
}

impl LifecycleLogs {
    pub fn connect(&mut self, endpoint: &str) {
        if self.endpoint.as_deref() != Some(endpoint) {
            *self = Self {
                endpoint: Some(endpoint.into()),
                ..Default::default()
            };
        }
    }

    pub fn disconnect(&mut self) {
        self.session = None;
        self.live = false;
        self.gap = true;
    }

    pub fn status(&self) -> &'static str {
        match (self.live, self.gap) {
            (true, false) => "Docker events live",
            (true, true) => "Docker events live · earlier gap possible",
            (false, true) => "Docker events reconnecting · gaps possible",
            (false, false) => "Watching Docker events…",
        }
    }
}

impl FerryDock {
    pub(super) fn lifecycle_event(&mut self, event: Event) {
        match event {
            Event::LifecycleSession(session) if self.connection.is_some() => {
                self.lifecycle_logs.session = Some(session);
            }
            Event::LifecycleStatus { session, live }
                if self.lifecycle_logs.session == Some(session) =>
            {
                self.lifecycle_logs.live = live;
                self.lifecycle_logs.gap |= !live;
            }
            Event::Lifecycle { session, marker }
                if self.lifecycle_logs.session == Some(session) =>
            {
                if self.lifecycle_logs.markers.iter().any(|old| {
                    old.container == marker.container
                        && old.timestamp == marker.timestamp
                        && old.action == marker.action
                }) {
                    return;
                }
                if self.projects.selected.is_none()
                    && self.containers.selected.as_deref() == Some(&marker.container)
                    && matches!(marker.action.as_str(), "start" | "restart")
                {
                    // A fast external restart can leave both inventory snapshots
                    // "running" even though the original log stream has ended.
                    self.container_logs.reopen = true;
                    self.backend.send(crate::docker::Command::Refresh);
                }
                self.containers.source_colors.register(&marker.name);
                self.lifecycle_logs.markers.push_back(marker);
                while self.lifecycle_logs.markers.len() > MAX_MARKERS {
                    self.lifecycle_logs.markers.pop_front();
                }
                self.annotate_container_logs();
                if let Some(project) = self.projects.selected.as_deref() {
                    self.project_logs.history.replace_annotations(
                        summarize(&self.lifecycle_logs.markers)
                            .into_iter()
                            .filter(|marker| marker.project.as_deref() == Some(project))
                            .map(|marker| (marker.project_line(), marker.details)),
                    );
                    if self.project_logs.follow {
                        self.project_logs.scroll.scroll_to_bottom();
                    }
                }
                if self.projects.selected.is_none() && self.container_logs.follow {
                    self.container_logs.scroll.scroll_to_bottom();
                }
            }
            _ => {}
        }
    }

    pub(super) fn annotate_container_logs(&mut self) {
        let id = self.containers.selected.as_deref();
        self.container_logs.buffer.set_annotations(
            summarize(&self.lifecycle_logs.markers)
                .iter()
                .filter(|marker| Some(marker.container.as_str()) == id)
                .map(Marker::styled)
                .collect(),
        );
    }

    pub(super) fn reset_container_log_buffer(&mut self) {
        self.container_logs.buffer = Buffer::for_docker_logs();
        self.annotate_container_logs();
    }
}

/// Kit's compact Tag fits the existing virtual log row; the tint and border
/// separate Docker annotations from application ANSI output in every theme.
pub(super) fn marker_row<T: gpui::Styled + gpui::ParentElement>(
    row: T,
    colors: super::ui::Palette,
) -> T {
    use gpui::{prelude::*, *};
    use gpui_kit::component::{Sizable, tag::Tag};
    row.flex()
        .items_center()
        .gap_2()
        .border_l_2()
        .border_color(rgb(colors.accent))
        .bg(rgba((colors.accent << 8) | 0x14))
        .px_2()
        .child(
            Tag::custom(
                rgb(colors.panel).into(),
                rgb(colors.accent).into(),
                rgb(colors.accent).into(),
            )
            .xsmall()
            .h(px(14.0))
            .py_0()
            .px_1()
            .flex_shrink_0()
            .child("DOCKER"),
        )
}
