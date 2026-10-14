//! Apply worker events and preserve shared session and selection invariants.
use super::FerryDock;
use super::ui::FeedbackNotification;
use crate::ansi::Buffer as StyledLogBuffer;
use crate::docker::Action;
use crate::docker::Command;
use crate::docker::Event;
use gpui::{prelude::*, *};
use gpui_kit::component::WindowExt as _;

impl FerryDock {
    pub(super) fn handle_event(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_services = matches!(
            &event,
            Event::Snapshot(_) | Event::ProjectLog { .. } | Event::ProjectLogBatch { .. }
        );
        let check_cleanup = matches!(
            &event,
            Event::Snapshot(_)
                | Event::ComposeProjects(_)
                | Event::ComposeConfigured { .. }
                | Event::Connected(_)
                | Event::Disconnected(_)
                | Event::ActionStarted { .. }
                | Event::ActionFinished { .. }
        );
        match event {
            event @ (Event::Volumes { .. }
            | Event::VolumeSizes { .. }
            | Event::VolumesPruned { .. }
            | Event::VolumeRemoved { .. }
            | Event::VolumesRemoved { .. }) => self.volume_event(event, window, cx),
            event @ (Event::Images { .. }
            | Event::ImageAnalysis { .. }
            | Event::ImageHistory { .. }
            | Event::ImageInspection { .. }
            | Event::ImageRemoved { .. }
            | Event::ImagesPruned { .. }
            | Event::ImagesRemoved { .. }) => self.image_event(event, window, cx),
            Event::MetricsSession(session) => {
                self.metrics.session = Some(session);
                self.metrics.containers.clear();
            }
            Event::Metrics {
                session,
                id,
                result,
            } if self.metrics.session == Some(session)
                && self.connection.is_some()
                && self
                    .containers
                    .items
                    .iter()
                    .any(|container| container.id == id && container.running()) =>
            {
                self.metrics.containers.entry(id).or_default().push(result);
            }
            Event::ProjectLogBatch { generation, lines }
                if generation == self.project_logs.generation
                    && self.projects.selected.is_some() =>
            {
                for line in lines {
                    self.containers.source_colors.register(&line.name);
                    self.project_logs.history.push(line);
                }
                if self.project_logs.follow {
                    self.project_logs.scroll.scroll_to_bottom();
                }
            }
            Event::ProjectLog { generation, line }
                if generation == self.project_logs.generation
                    && self.projects.selected.is_some() =>
            {
                self.containers.source_colors.register(&line.name);
                self.project_logs.history.push(line);
                if self.project_logs.follow {
                    self.project_logs.scroll.scroll_to_bottom();
                }
            }
            Event::ProjectLogsStatus { generation, status }
                if generation == self.project_logs.generation
                    && self.projects.selected.is_some() =>
            {
                self.project_logs.status = status
            }
            Event::ComposeProjects(projects) => self.projects.items = projects,
            Event::ComposeSettingsError(error) => {
                Self::notify(format!("Compose configuration: {error}"), true, window, cx)
            }
            Event::ComposeConfigured { project, result } => match result {
                Ok(config) => {
                    if let Some(existing) = self
                        .projects
                        .items
                        .iter_mut()
                        .find(|existing| existing.name == project)
                    {
                        existing.config = Some(config);
                    }
                    if self
                        .projects
                        .form
                        .as_ref()
                        .is_some_and(|form| form.project == project)
                    {
                        self.projects.form = None;
                    }
                    Self::notify(
                        format!("Compose configuration saved for {project}"),
                        false,
                        window,
                        cx,
                    );
                }
                Err(error) => {
                    if let Some(form) = &mut self.projects.form
                        && form.project == project
                    {
                        form.saving = false;
                        form.error = Some(error);
                    }
                }
            },
            Event::ComposeOutput {
                project,
                operation,
                data,
            } => {
                if let Some(current) = self.projects.operations.get_mut(&project)
                    && current.id == operation
                {
                    current.output.push(&data);
                    current.output_scroll.scroll_to_bottom();
                }
            }
            Event::ComposeFinished {
                project,
                operation,
                result,
            } => {
                if let Some(current) = self.projects.operations.get_mut(&project)
                    && current.id == operation
                {
                    let error = result.is_err();
                    let message = result
                        .as_ref()
                        .map(|()| format!("Compose {} completed", current.action.label()))
                        .unwrap_or_else(|error| error.clone());
                    current.running = false;
                    current.result = Some(result);
                    self.container_logs.reopen = true;
                    self.backend.send(Command::Refresh);
                    Self::notify(message, error, window, cx);
                }
            }
            Event::Connected(connection) => {
                self.volumes_connected(&connection.endpoint);
                self.connection = Some(connection);
                self.connection_error = None;
                if self.images_open || self.volumes_open {
                    self.refresh();
                }
                window.remove_notification::<FeedbackNotification>(cx);
                self.container_logs.reopen = true;
                if let Some(project) = self.projects.selected.clone() {
                    self.begin_project_logs(project);
                }
            }
            Event::Disconnected(error) => {
                self.volumes_disconnected(window, cx);
                self.images.generation += 1;
                self.invalidate_image_details(window, cx);
                self.images.loading = false;
                self.images.loaded = false;
                if self.images.confirmation.take().is_some() || self.images.prune_review {
                    window.close_dialog(cx);
                }
                self.images.interrupt_removal();
                self.metrics.session = None;
                self.container_logs.generation += 1;
                self.terminal.generation += 1;
                self.project_logs.generation += 1;
                self.project_logs.status =
                    "Docker disconnected · project logs reconnecting…".into();
                self.connection = None;
                self.connection_error = Some(error);
                self.containers.busy.clear();
                for operation in self
                    .projects
                    .operations
                    .values_mut()
                    .filter(|operation| operation.running)
                {
                    operation.running = false;
                    operation.result = Some(Err(
                        "Docker disconnected; check project state after reconnecting".into(),
                    ));
                }
                if let Some(form) = &mut self.projects.form {
                    form.saving = false;
                    form.error = Some("Docker disconnected; reconnect before saving".into());
                }
                self.container_logs.status = "Disconnected · reconnecting…".into();
                if let Some(terminal) = &self.terminal.view {
                    terminal.update(cx, |view, cx| {
                        view.ready = false;
                        view.status =
                            "Docker disconnected · reopen the shell after reconnecting".into();
                        cx.notify();
                    });
                }
            }
            Event::Snapshot(containers) => {
                self.metrics.containers.retain(|id, _| {
                    containers
                        .iter()
                        .any(|container| &container.id == id && container.running())
                });
                // Discovery order is stable on the first snapshot; later additions
                // never change existing colors, even when Docker IDs are replaced.
                let mut names = containers
                    .iter()
                    .map(|container| container.name.as_str())
                    .collect::<Vec<_>>();
                names.sort_unstable();
                for name in names {
                    self.containers.source_colors.register(name);
                }
                let before = self
                    .selected_container()
                    .map(|container| container.state.clone());
                let selected_exists = self
                    .containers
                    .selected
                    .as_ref()
                    .is_some_and(|id| containers.iter().any(|container| &container.id == id));
                self.containers.items = containers;
                self.project_logs.history.retain_sources(
                    self.containers
                        .items
                        .iter()
                        .map(|container| container.id.as_str()),
                );
                if !selected_exists {
                    self.containers.selected = None;
                    self.backend.send(Command::CloseLogs);
                    self.backend.send(Command::CloseTerminal);
                    self.terminal.view = None;
                    self.container_logs.buffer = StyledLogBuffer::for_docker_logs();
                    self.containers
                        .inspection
                        .update(cx, |editor, cx| editor.set_value("", window, cx));
                    self.container_logs.generation += 1;
                    let next = self
                        .containers
                        .items
                        .iter()
                        .find(|container| container.running())
                        .or_else(|| self.containers.items.first())
                        .map(|container| container.id.clone());
                    if let Some(id) = next
                        && self.projects.selected.is_none()
                    {
                        self.select(id, window, cx);
                    }
                } else {
                    let after = self
                        .selected_container()
                        .map(|container| container.state.clone());
                    if before != after || self.container_logs.reopen {
                        if let Some(id) = &self.containers.selected
                            && (after.as_deref() == Some("running") || self.container_logs.reopen)
                        {
                            self.container_logs.generation += 1;
                            self.container_logs.buffer = StyledLogBuffer::for_docker_logs();
                            self.backend.send(Command::Logs {
                                id: id.clone(),
                                generation: self.container_logs.generation,
                            });
                            self.backend.send(Command::Inspect {
                                id: id.clone(),
                                generation: self.container_logs.generation,
                            });
                        }
                        if after.as_deref() != Some("running") {
                            self.backend.send(Command::CloseTerminal);
                            if let Some(terminal) = &self.terminal.view {
                                terminal.update(cx, |view, cx| {
                                    view.ready = false;
                                    view.status =
                                        "Container stopped · start it to open a new shell".into();
                                    cx.notify();
                                });
                            }
                        }
                    }
                }
                self.container_logs.reopen = false;
            }
            Event::Log { generation, data } if generation == self.container_logs.generation => {
                self.container_logs.buffer.push(&data);
                self.container_logs.status = "Live · up to 1,500 lines retained".into();
                if self.container_logs.follow {
                    self.container_logs.scroll.scroll_to_bottom();
                }
            }
            Event::LogsEnded { generation, error }
                if generation == self.container_logs.generation =>
            {
                self.container_logs.status = error
                    .map(|error| format!("Logs unavailable: {error}"))
                    .unwrap_or_else(|| "Log stream ended · reconnect using Refresh logs".into());
            }
            Event::Inspection { generation, result }
                if generation == self.container_logs.generation =>
            {
                let text = result.unwrap_or_else(|error| format!("Inspection failed: {error}"));
                self.containers
                    .inspection
                    .update(cx, |editor, cx| editor.set_value(text, window, cx));
            }
            Event::ActionStarted { id, action } => {
                self.containers.busy.insert(id, action);
            }
            Event::ActionFinished { id, result } => {
                let action = self.containers.busy.remove(&id);
                let success = result.is_ok();
                let message = result.err().unwrap_or_default();
                let message = if success {
                    format!(
                        "{} completed",
                        action.map(Action::label).unwrap_or("Operation")
                    )
                } else {
                    message
                };
                Self::notify(message, !success, window, cx);
                if success && self.containers.selected.as_ref() == Some(&id) {
                    self.container_logs.reopen = true;
                }
                self.backend.send(Command::Refresh);
                if self.containers.selected.as_ref() == Some(&id) {
                    self.backend.send(Command::Inspect {
                        id,
                        generation: self.container_logs.generation,
                    });
                }
            }
            Event::TerminalReady { generation } if generation == self.terminal.generation => {
                if let Some(terminal) = &self.terminal.view {
                    terminal.update(cx, |view, cx| { view.ready = true; view.status = "Connected · Ctrl+Shift+C/V to copy/paste · Shift+PageUp/Down for scrollback".into(); cx.notify(); });
                }
            }
            Event::TerminalData { generation, data } if generation == self.terminal.generation => {
                if let Some(terminal) = &self.terminal.view {
                    terminal.update(cx, |view, cx| view.feed(&data, cx));
                }
            }
            Event::TerminalEnded { generation, error }
                if generation == self.terminal.generation =>
            {
                if let Some(terminal) = &self.terminal.view {
                    terminal.update(cx, |view, cx| {
                        view.ready = false;
                        view.status = error
                            .map(|error| format!("Shell failed: {error}"))
                            .unwrap_or_else(|| {
                                "Shell exited · reopen to start a new session".into()
                            });
                        cx.notify();
                    });
                }
            }
            _ => {}
        }
        if sync_services && self.projects.selected.is_some() {
            self.sync_service_filter(window, cx);
        }
        if check_cleanup {
            self.invalidate_cleanup_review(window, cx);
        }
    }
}
