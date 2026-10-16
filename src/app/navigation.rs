//! Keyboard shortcuts and navigation between feature panels.
use super::FerryDock;
use super::Tab;
use super::projects::ProjectTab;
use crate::docker::Command;
use crate::model::groups;
use gpui::{prelude::*, *};

impl FerryDock {
    pub(super) fn switch_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if tab != Tab::Compose {
            if self.projects.selected.is_some() {
                self.backend.send(Command::CloseProjectLogs);
                self.project_logs.generation += 1;
                if let Some(id) = self.containers.selected.clone() {
                    self.container_logs.generation += 1;
                    self.reset_container_log_buffer();
                    self.backend.send(Command::Logs {
                        id: id.clone(),
                        generation: self.container_logs.generation,
                    });
                }
            }
            self.projects.selected = None;
            self.projects.form = None;
        }
        self.tab = tab;
        if tab == Tab::Terminal {
            if let Some(terminal) = &self.terminal.view {
                terminal.update(cx, |view, cx| view.focus(window, cx));
            }
        } else if tab == Tab::Inspect {
            self.containers
                .inspection
                .focus_handle(cx)
                .focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn select_project(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_project_logs(project.clone());
        self.projects.selected = Some(project);
        self.projects.tab = ProjectTab::Logs;
        self.project_logs.service = None;
        self.project_logs.follow = true;
        self.sync_service_filter(window, cx);
        self.projects.form = None;
        self.tab = Tab::Compose;
        self.expand_dock(380.0, window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn search_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event.keystroke.key.as_str(), "escape" | "enter") {
            self.focus.focus(window, cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn root_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "linux")]
        if self.updates.applying {
            cx.stop_propagation();
            return;
        }
        let key = &event.keystroke;
        if self.favorites_dialog_open() || self.quick_surface_open() {
            return;
        }
        if self.projects.cleanup.is_some() {
            return;
        }
        if key.modifiers.control && key.key == "," {
            self.show_settings(!self.settings_open, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.settings_open {
            return;
        }
        if self.networks_open {
            self.networks_key(event, window, cx);
            return;
        }
        if self.volumes_open {
            self.volumes_key(event, window, cx);
            return;
        }
        if self.images_open {
            self.images_key(event, window, cx);
            return;
        }
        match key.key.as_str() {
            "enter" if self.focus.is_focused(window) && self.projects.selected.is_none() => {
                self.open_terminal(window, cx)
            }
            "f" if key.modifiers.control => {
                self.containers.search_focus.focus(window, cx);
                cx.stop_propagation();
            }
            "r" if key.modifiers.control => {
                self.backend.send(Command::Refresh);
                cx.stop_propagation();
            }
            "1" if key.modifiers.control => self.switch_tab(Tab::Logs, window, cx),
            "2" if key.modifiers.control => self.switch_tab(Tab::Terminal, window, cx),
            "3" if key.modifiers.control => self.switch_tab(Tab::Inspect, window, cx),
            "4" if key.modifiers.control => {
                if let Some(project) = self
                    .selected_container()
                    .and_then(|container| container.project.clone())
                {
                    self.select_project(project, window, cx);
                }
                cx.stop_propagation();
            }
            "up" | "down" if self.focus.is_focused(window) => {
                let items = groups(
                    &self.containers.items,
                    self.containers.running_only,
                    &self.containers.filter,
                )
                .into_iter()
                .filter(|(group, _)| self.containers.group_expanded(group))
                .flat_map(|(_, members)| members)
                .collect::<Vec<_>>();
                let current = items
                    .iter()
                    .position(|item| Some(&item.id) == self.containers.selected.as_ref());
                let next = match (current, key.key.as_str()) {
                    (Some(current), "up") => current.saturating_sub(1),
                    (Some(current), _) => (current + 1).min(items.len().saturating_sub(1)),
                    (None, "up") => items.len().saturating_sub(1),
                    (None, _) => 0,
                };
                if let Some(container) = items.get(next) {
                    self.select(container.id.clone(), window, cx);
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}
