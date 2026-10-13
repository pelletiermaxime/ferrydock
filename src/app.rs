use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};

use crate::{
    compose::{self, Config as ComposeConfig, Project as ComposeProject},
    docker::{Action, Backend, Command, Connection, Event, Shell},
    model::{Container, GroupKey, LogBuffer, groups},
    terminal::TerminalView,
};

const BG: u32 = 0x171d21;
const PANEL: u32 = 0x1e272c;
const TEXT: u32 = 0xe0ece7;
const MUTED: u32 = 0x9eb2a8;
const BORDER: u32 = 0x34433c;
const ACCENT: u32 = 0x74d7b1;
const SELECTED: u32 = 0x234136;
const ERROR: u32 = 0xffa4a4;

enum ResourceRow {
    Heading(GroupKey, String),
    Container(Box<Container>),
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Logs,
    Terminal,
    Inspect,
    Compose,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Terminal => "Terminal",
            Self::Inspect => "Inspect",
            Self::Compose => "Project",
        }
    }
}

struct ComposeForm {
    project: String,
    fields: [String; 4],
    active: usize,
    saving: bool,
    error: Option<String>,
}

struct ComposeOperation {
    id: u64,
    action: compose::Action,
    running: bool,
    output: LogBuffer,
    result: Option<Result<(), String>>,
}

#[derive(Clone, Copy, PartialEq)]
enum ProjectTab {
    Logs,
    Operations,
}

pub struct FerryDock {
    backend: Backend,
    events: tokio::sync::mpsc::Receiver<Event>,
    connection: Option<Connection>,
    connection_error: Option<String>,
    containers: Vec<Container>,
    selected: Option<String>,
    generation: u64,
    terminal_generation: u64,
    terminal: Option<Entity<TerminalView>>,
    tab: Tab,
    shell: Shell,
    logs: LogBuffer,
    log_status: String,
    log_follow: bool,
    log_scroll: UniformListScrollHandle,
    inspection: String,
    busy: HashMap<String, String>,
    message: Option<(String, bool)>,
    running_only: bool,
    filter: String,
    search_focus: FocusHandle,
    focus: FocusHandle,
    dock_height: f32,
    resize_origin: Option<(f32, f32)>,
    reopen_logs: bool,
    projects: Vec<ComposeProject>,
    selected_project: Option<String>,
    compose_form: Option<ComposeForm>,
    compose_focus: FocusHandle,
    compose_operations: HashMap<String, ComposeOperation>,
    next_operation: u64,
    project_tab: ProjectTab,
    project_log_generation: u64,
    project_logs: crate::project_logs::History,
    project_log_status: String,
    project_log_follow: bool,
    project_log_scroll: UniformListScrollHandle,
    project_service: Option<String>,
    resource_scroll: UniformListScrollHandle,
    #[cfg(all(test, feature = "headless-tests"))]
    project_rows_rendered: std::cell::Cell<usize>,
}

impl FerryDock {
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (backend, events) = Backend::start();
        Self::with_backend(backend, events, cx)
    }

    fn with_backend(
        backend: Backend,
        events: tokio::sync::mpsc::Receiver<Event>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let mut changed = false;
                        let started = Instant::now();
                        for _ in 0..64 {
                            match this.events.try_recv() {
                                Ok(event) => {
                                    this.handle_event(event, cx);
                                    changed = true;
                                }
                                Err(_) => break,
                            }
                            if started.elapsed() >= Duration::from_millis(8) {
                                break;
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            backend,
            events,
            connection: None,
            connection_error: None,
            containers: Vec::new(),
            selected: None,
            generation: 0,
            terminal_generation: 0,
            terminal: None,
            tab: Tab::Logs,
            shell: Shell::Auto,
            logs: LogBuffer::default(),
            log_status: "Select a container to view its logs".into(),
            log_follow: true,
            log_scroll: UniformListScrollHandle::default(),
            inspection: String::new(),
            busy: HashMap::new(),
            message: None,
            running_only: true,
            filter: String::new(),
            search_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            dock_height: 340.0,
            resize_origin: None,
            reopen_logs: true,
            projects: Vec::new(),
            selected_project: None,
            compose_form: None,
            compose_focus: cx.focus_handle(),
            compose_operations: HashMap::new(),
            next_operation: 0,
            project_tab: ProjectTab::Logs,
            project_log_generation: 0,
            project_logs: Default::default(),
            project_log_status: "Select a Compose project to view its logs".into(),
            project_log_follow: true,
            project_log_scroll: UniformListScrollHandle::default(),
            project_service: None,
            resource_scroll: UniformListScrollHandle::default(),
            #[cfg(all(test, feature = "headless-tests"))]
            project_rows_rendered: std::cell::Cell::new(0),
        }
    }

    fn selected_container(&self) -> Option<&Container> {
        self.selected
            .as_ref()
            .and_then(|id| self.containers.iter().find(|container| &container.id == id))
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        let returning_from_project = self.selected_project.is_some();
        if self.selected_project.is_some() {
            self.backend.send(Command::CloseProjectLogs);
            self.project_log_generation += 1;
        }
        self.selected_project = None;
        self.compose_form = None;
        if self.tab == Tab::Compose {
            self.tab = Tab::Logs;
        }
        if self.selected.as_ref() == Some(&id) && !returning_from_project {
            return;
        }
        if self.selected.as_ref() != Some(&id) {
            self.backend.send(Command::CloseTerminal);
            self.terminal = None;
        }
        self.selected = Some(id.clone());
        self.generation += 1;
        self.logs = LogBuffer::default();
        self.inspection = "Loading container configuration…".into();
        self.log_status = "Connecting to logs…".into();
        self.log_follow = true;
        self.backend.send(Command::Logs {
            id: id.clone(),
            generation: self.generation,
        });
        self.backend.send(Command::Inspect {
            id,
            generation: self.generation,
        });
        // Avoid opening another interactive process merely by changing rows.
        if self.tab == Tab::Terminal {
            self.tab = Tab::Logs;
        }
        cx.notify();
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::ProjectLogBatch { generation, lines }
                if generation == self.project_log_generation && self.selected_project.is_some() =>
            {
                for line in lines {
                    self.project_logs.push(line);
                }
                if self.project_log_follow {
                    self.project_log_scroll.scroll_to_bottom();
                }
            }
            Event::ProjectLog { generation, line }
                if generation == self.project_log_generation && self.selected_project.is_some() =>
            {
                self.project_logs.push(line);
                if self.project_log_follow {
                    self.project_log_scroll.scroll_to_bottom();
                }
            }
            Event::ProjectLogsStatus { generation, status }
                if generation == self.project_log_generation && self.selected_project.is_some() =>
            {
                self.project_log_status = status
            }
            Event::ComposeProjects(projects) => self.projects = projects,
            Event::ComposeSettingsError(error) => {
                self.message = Some((format!("Compose configuration: {error}"), true))
            }
            Event::ComposeConfigured { project, result } => match result {
                Ok(config) => {
                    if let Some(existing) = self
                        .projects
                        .iter_mut()
                        .find(|existing| existing.name == project)
                    {
                        existing.config = Some(config);
                    }
                    if self
                        .compose_form
                        .as_ref()
                        .is_some_and(|form| form.project == project)
                    {
                        self.compose_form = None;
                    }
                    self.message =
                        Some((format!("Compose configuration saved for {project}"), false));
                }
                Err(error) => {
                    if let Some(form) = &mut self.compose_form
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
                if let Some(current) = self.compose_operations.get_mut(&project)
                    && current.id == operation
                {
                    current.output.push(&data);
                }
            }
            Event::ComposeFinished {
                project,
                operation,
                result,
            } => {
                if let Some(current) = self.compose_operations.get_mut(&project)
                    && current.id == operation
                {
                    current.running = false;
                    current.result = Some(result);
                    self.reopen_logs = true;
                    self.backend.send(Command::Refresh);
                }
            }
            Event::Connected(connection) => {
                self.connection = Some(connection);
                self.connection_error = None;
                self.message = None;
                self.reopen_logs = true;
                if let Some(project) = self.selected_project.clone() {
                    self.begin_project_logs(project);
                }
            }
            Event::Disconnected(error) => {
                self.project_log_generation += 1;
                self.project_log_status = "Docker disconnected · project logs reconnecting…".into();
                self.connection = None;
                self.connection_error = Some(error);
                self.busy.clear();
                for operation in self
                    .compose_operations
                    .values_mut()
                    .filter(|operation| operation.running)
                {
                    operation.running = false;
                    operation.result = Some(Err(
                        "Docker disconnected; check project state after reconnecting".into(),
                    ));
                }
                if let Some(form) = &mut self.compose_form {
                    form.saving = false;
                    form.error = Some("Docker disconnected; reconnect before saving".into());
                }
                self.log_status = "Disconnected · reconnecting…".into();
                if let Some(terminal) = &self.terminal {
                    terminal.update(cx, |view, cx| {
                        view.ready = false;
                        view.status =
                            "Docker disconnected · reopen the shell after reconnecting".into();
                        cx.notify();
                    });
                }
            }
            Event::Snapshot(containers) => {
                let before = self
                    .selected_container()
                    .map(|container| container.state.clone());
                let selected_exists = self
                    .selected
                    .as_ref()
                    .is_some_and(|id| containers.iter().any(|container| &container.id == id));
                self.containers = containers;
                if !selected_exists {
                    self.selected = None;
                    self.backend.send(Command::CloseLogs);
                    self.backend.send(Command::CloseTerminal);
                    self.terminal = None;
                    self.logs = LogBuffer::default();
                    self.inspection.clear();
                    self.generation += 1;
                    let next = self
                        .containers
                        .iter()
                        .find(|container| container.running())
                        .or_else(|| self.containers.first())
                        .map(|container| container.id.clone());
                    if let Some(id) = next
                        && self.selected_project.is_none()
                    {
                        self.select(id, cx);
                    }
                } else {
                    let after = self
                        .selected_container()
                        .map(|container| container.state.clone());
                    if before != after || self.reopen_logs {
                        if let Some(id) = &self.selected
                            && (after.as_deref() == Some("running") || self.reopen_logs)
                        {
                            self.generation += 1;
                            self.logs = LogBuffer::default();
                            self.backend.send(Command::Logs {
                                id: id.clone(),
                                generation: self.generation,
                            });
                            self.backend.send(Command::Inspect {
                                id: id.clone(),
                                generation: self.generation,
                            });
                        }
                        if after.as_deref() != Some("running") {
                            self.backend.send(Command::CloseTerminal);
                            if let Some(terminal) = &self.terminal {
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
                self.reopen_logs = false;
            }
            Event::Log { generation, data } if generation == self.generation => {
                self.logs.push(&data);
                self.log_status = "Live · timestamps enabled · up to 1,500 lines retained".into();
                if self.log_follow {
                    self.log_scroll.scroll_to_bottom();
                }
            }
            Event::LogsEnded { generation, error } if generation == self.generation => {
                self.log_status = error
                    .map(|error| format!("Logs unavailable: {error}"))
                    .unwrap_or_else(|| "Log stream ended · reconnect using Refresh logs".into());
            }
            Event::Inspection { generation, result } if generation == self.generation => {
                self.inspection =
                    result.unwrap_or_else(|error| format!("Inspection failed: {error}"));
            }
            Event::ActionStarted { id, action } => {
                self.busy
                    .insert(id, format!("{} in progress…", action.label()));
            }
            Event::ActionFinished { id, result } => {
                let action = self.busy.remove(&id).unwrap_or_else(|| "Operation".into());
                let success = result.is_ok();
                self.message = Some(match result {
                    Ok(()) => (
                        format!("{} completed", action.trim_end_matches(" in progress…")),
                        false,
                    ),
                    Err(error) => (error, true),
                });
                if success && self.selected.as_ref() == Some(&id) {
                    self.reopen_logs = true;
                }
                self.backend.send(Command::Refresh);
                if self.selected.as_ref() == Some(&id) {
                    self.backend.send(Command::Inspect {
                        id,
                        generation: self.generation,
                    });
                }
            }
            Event::TerminalReady { generation } if generation == self.terminal_generation => {
                if let Some(terminal) = &self.terminal {
                    terminal.update(cx, |view, cx| { view.ready = true; view.status = "Connected · Ctrl+Shift+C/V to copy/paste · Shift+PageUp/Down for scrollback".into(); cx.notify(); });
                }
            }
            Event::TerminalData { generation, data } if generation == self.terminal_generation => {
                if let Some(terminal) = &self.terminal {
                    terminal.update(cx, |view, cx| view.feed(&data, cx));
                }
            }
            Event::TerminalEnded { generation, error }
                if generation == self.terminal_generation =>
            {
                if let Some(terminal) = &self.terminal {
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
    }

    fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(container) = self.selected_container() else {
            return;
        };
        if !container.running() || self.connection.is_none() {
            return;
        }
        let id = container.id.clone();
        self.terminal_generation += 1;
        let generation = self.terminal_generation;
        let commands = self.backend.commands.clone();
        let view = cx.new(|cx| TerminalView::new(generation, commands, cx));
        view.update(cx, |view, cx| view.focus(window, cx));
        self.terminal = Some(view);
        self.backend.send(Command::Terminal {
            id,
            generation,
            shell: self.shell,
            columns: 80,
            rows: 24,
        });
        self.tab = Tab::Terminal;
        self.dock_height = self.dock_height.max(460.0);
        cx.notify();
    }

    fn switch_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if tab != Tab::Compose {
            if self.selected_project.is_some() {
                self.backend.send(Command::CloseProjectLogs);
                self.project_log_generation += 1;
                if let Some(id) = &self.selected {
                    self.generation += 1;
                    self.logs = Default::default();
                    self.backend.send(Command::Logs {
                        id: id.clone(),
                        generation: self.generation,
                    });
                }
            }
            self.selected_project = None;
            self.compose_form = None;
        }
        self.tab = tab;
        if tab == Tab::Terminal {
            if let Some(terminal) = &self.terminal {
                terminal.update(cx, |view, cx| view.focus(window, cx));
            }
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn action(&mut self, action: Action, cx: &mut Context<Self>) {
        let Some(container) = self.selected_container() else {
            return;
        };
        if self.connection.is_none()
            || self.busy.contains_key(&container.id)
            || container
                .project
                .as_ref()
                .is_some_and(|project| self.project_busy(project))
        {
            return;
        }
        let id = container.id.clone();
        if matches!(action, Action::Stop | Action::Restart) {
            self.backend.send(Command::CloseTerminal);
            if let Some(view) = &self.terminal {
                view.update(cx, |view, cx| {
                    view.ready = false;
                    view.status = "Container operation closed this shell session".into();
                    cx.notify();
                });
            }
        }
        self.busy
            .insert(id.clone(), format!("{} in progress…", action.label()));
        self.message = None;
        self.backend.send(Command::Action { id, action });
        cx.notify();
    }

    fn project_busy(&self, project: &str) -> bool {
        self.compose_operations
            .get(project)
            .is_some_and(|operation| operation.running)
            || self.containers.iter().any(|container| {
                container.project.as_deref() == Some(project)
                    && self.busy.contains_key(&container.id)
            })
    }

    fn select_project(&mut self, project: String, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_project_logs(project.clone());
        self.selected_project = Some(project);
        self.project_tab = ProjectTab::Logs;
        self.project_service = None;
        self.project_log_follow = true;
        self.compose_form = None;
        self.tab = Tab::Compose;
        self.dock_height = self.dock_height.max(380.0);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn begin_project_logs(&mut self, project: String) {
        self.project_log_generation += 1;
        self.project_logs = Default::default();
        self.project_log_status = "Connecting to project logs…".into();
        self.backend.send(Command::CloseLogs);
        self.backend.send(Command::ProjectLogs {
            project,
            generation: self.project_log_generation,
        });
    }

    fn configure_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .projects
            .iter()
            .find(|project| Some(&project.name) == self.selected_project.as_ref())
        else {
            return;
        };
        if self.project_busy(&project.name) {
            return;
        }
        let config = project.config.as_ref().unwrap_or(&project.hint);
        let paths = |paths: &[std::path::PathBuf]| {
            paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.compose_form = Some(ComposeForm {
            project: project.name.clone(),
            fields: [
                config.directory.to_string_lossy().into_owned(),
                paths(&config.files),
                paths(&config.env_files),
                config.profiles.join("\n"),
            ],
            active: 0,
            saving: false,
            error: None,
        });
        self.dock_height = self.dock_height.max(500.0);
        self.compose_focus.focus(window, cx);
        cx.notify();
    }

    fn save_project(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.compose_form else {
            return;
        };
        if form.saving || self.connection.is_none() || self.project_busy(&form.project) {
            return;
        }
        let paths = |text: &str| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(std::path::PathBuf::from)
                .collect()
        };
        let config = ComposeConfig {
            project: form.project.clone(),
            directory: form.fields[0].clone().into(),
            files: paths(&form.fields[1]),
            env_files: paths(&form.fields[2]),
            profiles: form.fields[3]
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect(),
        };
        let form = self.compose_form.as_mut().unwrap();
        form.saving = true;
        form.error = None;
        self.backend.send(Command::ConfigureCompose { config });
        cx.notify();
    }

    fn compose_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.compose_form else {
            return;
        };
        if form.saving {
            cx.stop_propagation();
            return;
        }
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" => {
                self.compose_form = None;
                self.focus.focus(window, cx);
            }
            "tab" => form.active = (form.active + if key.modifiers.shift { 3 } else { 1 }) % 4,
            "backspace" => {
                form.fields[form.active].pop();
            }
            "enter" if form.active > 0 => form.fields[form.active].push('\n'),
            "a" if key.modifiers.control => form.fields[form.active].clear(),
            "v" if key.modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    form.fields[form.active].push_str(&text.replace('\r', ""));
                }
            }
            _ if !key.modifiers.control && !key.modifiers.alt => {
                if let Some(text) = &key.key_char {
                    form.fields[form.active].push_str(text);
                }
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn compose_action(&mut self, action: compose::Action, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project.clone() else {
            return;
        };
        if self.connection.is_none()
            || self.project_busy(&project)
            || self.compose_form.is_some()
            || !self
                .projects
                .iter()
                .any(|item| item.name == project && item.config.is_some())
        {
            return;
        }
        self.next_operation += 1;
        self.project_tab = ProjectTab::Operations;
        self.compose_operations.insert(
            project.clone(),
            ComposeOperation {
                id: self.next_operation,
                action,
                running: true,
                output: LogBuffer::default(),
                result: None,
            },
        );
        if matches!(action, compose::Action::Stop | compose::Action::Restart)
            && self
                .selected_container()
                .is_some_and(|container| container.project.as_ref() == Some(&project))
        {
            self.backend.send(Command::CloseTerminal);
            if let Some(terminal) = &self.terminal {
                terminal.update(cx, |view, cx| {
                    view.ready = false;
                    view.status = "Compose operation closed this shell session".into();
                    cx.notify();
                });
            }
        }
        self.backend.send(Command::Compose {
            project,
            operation: self.next_operation,
            action,
        });
        cx.notify();
    }

    fn search_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" | "enter" => self.focus.focus(window, cx),
            "backspace" => {
                self.filter.pop();
            }
            "v" if key.modifiers.control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.filter.push_str(&text.replace(['\r', '\n'], " "));
                }
            }
            "a" if key.modifiers.control => self.filter.clear(),
            _ if !key.modifiers.control && !key.modifiers.alt => {
                if let Some(text) = &key.key_char {
                    self.filter.push_str(text);
                }
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn root_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "enter" if self.selected_project.is_none() => self.open_terminal(window, cx),
            "f" if key.modifiers.control => {
                self.search_focus.focus(window, cx);
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
            "up" | "down" => {
                let items = groups(&self.containers, self.running_only, &self.filter)
                    .into_values()
                    .flatten()
                    .collect::<Vec<_>>();
                let current = items
                    .iter()
                    .position(|item| Some(&item.id) == self.selected.as_ref())
                    .unwrap_or(0);
                let next = if key.key == "up" {
                    current.saturating_sub(1)
                } else {
                    (current + 1).min(items.len().saturating_sub(1))
                };
                if let Some(container) = items.get(next) {
                    self.select(container.id.clone(), cx);
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    fn button(
        id: impl Into<SharedString>,
        text: impl Into<SharedString>,
        active: bool,
        disabled: bool,
    ) -> Stateful<Div> {
        let id = id.into();
        div()
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(if active { SELECTED } else { PANEL }))
            .text_color(rgb(if active { ACCENT } else { TEXT }))
            .text_size(px(12.0))
            .cursor(if disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            })
            .when(disabled, |button| button.opacity(0.45))
            .when(!disabled, |button| {
                button.hover(|style| style.bg(rgb(SELECTED)))
            })
            .child(text.into())
    }

    fn resources(&self, cx: &mut Context<Self>) -> UniformList {
        let mut groups = groups(&self.containers, self.running_only, &self.filter);
        for project in &self.projects {
            if self.filter.is_empty()
                || project
                    .name
                    .to_lowercase()
                    .contains(&self.filter.to_lowercase())
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
        uniform_list(
            "containers-scroll",
            rows.len().max(1),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| match rows.get(index) {
                        Some(ResourceRow::Heading(project, count_label)) => {
                            let selected = matches!(project, GroupKey::Compose(name)
                                if this.selected_project.as_ref() == Some(name));
                            let compose_project = match project {
                                GroupKey::Compose(name) => Some(name.clone()),
                                GroupKey::Standalone => None,
                            };
                            div()
                                .h(px(56.0))
                                .w_full()
                                .id(SharedString::from(format!("project-{}", project.label())))
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
                                .bg(rgb(if selected { SELECTED } else { BG }))
                                .text_size(px(12.0))
                                .when_some(compose_project, |header, name| {
                                    header
                                        .cursor_pointer()
                                        .hover(move |style| {
                                            style.bg(rgb(if selected { SELECTED } else { PANEL }))
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.select_project(name.clone(), window, cx)
                                        }))
                                })
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(project.label().to_owned()),
                                )
                                .child(div().text_color(rgb(MUTED)).child(count_label.clone()))
                                .into_any_element()
                        }
                        Some(ResourceRow::Container(container)) => {
                            let selected = this.selected_project.is_none()
                                && this.selected.as_ref() == Some(&container.id);
                            let running = container.running();
                            let id = container.id.clone();
                            let busy = this.busy.get(&container.id).cloned();
                            div()
                                .id(SharedString::from(format!("row-{}", container.id)))
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
                                .bg(rgb(if selected { SELECTED } else { BG }))
                                .hover(move |style| {
                                    style.bg(rgb(if selected { SELECTED } else { PANEL }))
                                })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select(id.clone(), cx);
                                    this.focus.focus(window, cx);
                                }))
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
                                                .text_color(rgb(MUTED))
                                                .text_ellipsis()
                                                .child(container.image.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(160.0))
                                        .text_size(px(12.0))
                                        .text_color(rgb(if running { ACCENT } else { MUTED }))
                                        .child(
                                            busy.unwrap_or_else(|| {
                                                format!("● {}", container.state)
                                            }),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(rgb(MUTED))
                                                .text_ellipsis()
                                                .child(container.status.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .w(px(140.0))
                                        .text_size(px(12.0))
                                        .text_color(rgb(MUTED))
                                        .text_ellipsis()
                                        .child(if container.ports.is_empty() {
                                            "—".into()
                                        } else {
                                            container.ports.clone()
                                        }),
                                )
                                .into_any_element()
                        }
                        None => div()
                            .h(px(56.0))
                            .p_3()
                            .text_color(rgb(MUTED))
                            .child(if this.connection.is_none() {
                                "Waiting for Docker…"
                            } else if !this.filter.is_empty() {
                                "No containers match your search"
                            } else if this.running_only {
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
        .track_scroll(&self.resource_scroll)
        .flex_1()
        .min_h_0()
        .p_3()
    }

    fn logs_view(&self, cx: &mut Context<Self>) -> Div {
        let count = self.logs.lines.len() + usize::from(!self.logs.partial.is_empty());
        let widest = self
            .logs
            .lines
            .iter()
            .map(String::len)
            .chain(std::iter::once(self.logs.partial.len()))
            .enumerate()
            .max_by_key(|(_, length)| *length)
            .map(|(index, _)| index);
        let lines = uniform_list(
            "log-scroll",
            count.max(1),
            cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                range
                    .map(|index| {
                        div().h(px(18.0)).whitespace_nowrap().child(
                            this.logs.lines.get(index).cloned().unwrap_or_else(|| {
                                if this.logs.partial.is_empty() {
                                    "Waiting for log output…".into()
                                } else {
                                    this.logs.partial.clone()
                                }
                            }),
                        )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .with_width_from_item(widest)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(&self.log_scroll)
        .flex_1()
        .min_h_0()
        .p_3()
        .font_family("DejaVu Sans Mono")
        .text_size(px(12.0));
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x101819))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        Self::button(
                            "follow",
                            if self.log_follow {
                                "Following"
                            } else {
                                "Follow paused"
                            },
                            self.log_follow,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.log_follow = !this.log_follow;
                            if this.log_follow {
                                this.log_scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::button("copy-logs", "Copy logs", false, false).on_click(cx.listener(
                            |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(this.logs.text()));
                                this.message = Some(("Retained logs copied".into(), false));
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Self::button(
                            "refresh-logs",
                            "Refresh logs",
                            false,
                            self.connection.is_none(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = &this.selected
                                && this.connection.is_some()
                            {
                                this.generation += 1;
                                this.logs = LogBuffer::default();
                                this.backend.send(Command::Logs {
                                    id: id.clone(),
                                    generation: this.generation,
                                });
                                this.backend.send(Command::Inspect {
                                    id: id.clone(),
                                    generation: this.generation,
                                });
                                this.log_status = "Connecting to logs…".into();
                                cx.notify();
                            }
                        })),
                    ),
            )
            .child(lines)
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child(self.log_status.clone()),
            )
    }

    fn terminal_view(&self, cx: &mut Context<Self>) -> Div {
        let running =
            self.selected_container().is_some_and(Container::running) && self.connection.is_some();
        let ready = self
            .terminal
            .as_ref()
            .is_some_and(|view| view.read(cx).ready);
        let mut toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER));
        toolbar = toolbar.child(
            Self::button(
                "open-shell",
                if self.terminal.is_none() {
                    "Open shell"
                } else if ready {
                    "Connected"
                } else {
                    "Reopen shell"
                },
                ready,
                !running || ready,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if !this
                    .terminal
                    .as_ref()
                    .is_some_and(|view| view.read(cx).ready)
                {
                    this.open_terminal(window, cx);
                }
            })),
        );
        for shell in [Shell::Auto, Shell::Bash, Shell::Sh] {
            let active = std::mem::discriminant(&self.shell) == std::mem::discriminant(&shell);
            let id = match shell {
                Shell::Auto => "shell-auto",
                Shell::Bash => "shell-bash",
                Shell::Sh => "shell-sh",
            };
            toolbar = toolbar.child(Self::button(id, shell.label(), active, ready).on_click(
                cx.listener(move |this, _, _, cx| {
                    if !this
                        .terminal
                        .as_ref()
                        .is_some_and(|view| view.read(cx).ready)
                    {
                        this.shell = shell;
                        cx.notify();
                    }
                }),
            ));
        }
        toolbar = toolbar
            .child(div().flex_1())
            .child(
                Self::button("paste-terminal", "Paste", false, !ready).on_click(cx.listener(
                    |this, _, window, cx| {
                        if let Some(view) = &this.terminal {
                            view.update(cx, |view, cx| {
                                view.paste(cx);
                                view.focus(window, cx);
                            });
                        }
                    },
                )),
            )
            .child(
                Self::button(
                    "disconnect-terminal",
                    "Disconnect",
                    false,
                    self.terminal.is_none(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.backend.send(Command::CloseTerminal);
                    this.terminal_generation += 1;
                    this.terminal = None;
                    cx.notify();
                })),
            );
        let mut pane = div().flex().flex_col().size_full().child(toolbar);
        if let Some(terminal) = &self.terminal {
            let status = terminal.read(cx).status.clone();
            pane = pane
                .child(div().flex_1().min_h_0().child(terminal.clone()))
                .child(
                    div()
                        .px_3()
                        .py_1()
                        .text_size(px(11.0))
                        .text_color(rgb(MUTED))
                        .child(status),
                );
        } else {
            pane = pane.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(MUTED))
                    .child(if running {
                        "Open a shell inside this container"
                    } else {
                        "Start this container to open a shell"
                    }),
            );
        }
        pane
    }

    fn detail(&self, cx: &mut Context<Self>) -> Div {
        if let Some(project) = &self.selected_project {
            return self.project_detail(project, cx);
        }
        let Some(container) = self.selected_container() else {
            return div()
                .h(px(self.dock_height))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(MUTED))
                .child("Select a container to view logs, open a shell, or inspect it");
        };
        let unavailable = self.connection.is_none()
            || self.busy.contains_key(&container.id)
            || container
                .project
                .as_ref()
                .is_some_and(|project| self.project_busy(project));
        let primary = if container.running() {
            Action::Stop
        } else {
            Action::Start
        };
        let mut detail = div()
            .h(px(self.dock_height))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .border_t_1()
            .border_color(rgb(BORDER));
        detail = detail.child(
            div()
                .id("dock-resize")
                .h(px(6.0))
                .cursor(CursorStyle::ResizeUpDown)
                .hover(|style| style.bg(rgb(ACCENT)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        this.resize_origin = Some((f32::from(event.position.y), this.dock_height));
                        cx.notify();
                    }),
                ),
        );
        detail =
            detail.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(container.name.clone())
                            .child(div().text_size(px(11.0)).text_color(rgb(MUTED)).child(
                                format!(
                                    "{} · {}",
                                    container.project.as_deref().unwrap_or("Standalone"),
                                    &container.id[..container.id.len().min(12)]
                                ),
                            )),
                    )
                    .child(
                        Self::button("primary-action", primary.label(), false, unavailable)
                            .on_click(cx.listener(move |this, _, _, cx| this.action(primary, cx))),
                    )
                    .child(
                        Self::button(
                            "restart",
                            "Restart",
                            false,
                            unavailable || !container.running(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.selected_container().is_some_and(Container::running) {
                                this.action(Action::Restart, cx);
                            }
                        })),
                    ),
            );
        let mut tabs = div()
            .flex()
            .gap_5()
            .px_4()
            .border_b_1()
            .border_color(rgb(BORDER));
        for tab in [Tab::Logs, Tab::Terminal, Tab::Inspect] {
            tabs = tabs.child(
                div()
                    .id(tab.name())
                    .py_2()
                    .border_b_2()
                    .border_color(rgb(if self.tab == tab { ACCENT } else { BG }))
                    .text_color(rgb(if self.tab == tab { ACCENT } else { MUTED }))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .child(tab.name())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.switch_tab(tab, window, cx)),
                    ),
            );
        }
        detail.child(tabs).child(
            div().flex_1().min_h_0().child(match self.tab {
                Tab::Logs => self.logs_view(cx).into_any_element(),
                Tab::Terminal => self.terminal_view(cx).into_any_element(),
                Tab::Inspect => div()
                    .id("inspection")
                    .size_full()
                    .overflow_y_scroll()
                    .overflow_x_scroll()
                    .p_3()
                    .font_family("DejaVu Sans Mono")
                    .text_size(px(12.0))
                    .child(self.inspection.clone())
                    .into_any_element(),
                Tab::Compose => div().into_any_element(),
            }),
        )
    }

    fn project_detail(&self, name: &str, cx: &mut Context<Self>) -> Div {
        let project = self.projects.iter().find(|project| project.name == name);
        let configured = project.is_some_and(|project| project.config.is_some());
        let busy = self.project_busy(name);
        let disabled =
            !configured || busy || self.connection.is_none() || self.compose_form.is_some();
        let total = self
            .containers
            .iter()
            .filter(|container| container.project.as_deref() == Some(name))
            .count();
        let running = self
            .containers
            .iter()
            .filter(|container| container.project.as_deref() == Some(name) && container.running())
            .count();
        let mut toolbar = div().flex().items_center().gap_2().px_4().py_2().border_b_1().border_color(rgb(BORDER))
            .child(div().flex_1().min_w_0().child(format!("Compose · {name}"))
                .child(div().text_size(px(11.0)).text_color(rgb(MUTED)).child(format!("{running} running / {total} containers · actions apply to the configured project and profiles"))))
            .child(Self::button("compose-configure", if configured { "Configuration" } else { "Associate files" }, false, busy || self.connection.is_none())
                .on_click(cx.listener(|this, _, window, cx| { if this.connection.is_some() { this.configure_project(window, cx); } })));
        for (action, id) in [
            (compose::Action::Start, "compose-start"),
            (compose::Action::Stop, "compose-stop"),
            (compose::Action::Restart, "compose-restart"),
        ] {
            toolbar = toolbar.child(
                Self::button(id, action.label(), false, disabled)
                    .on_click(cx.listener(move |this, _, _, cx| this.compose_action(action, cx))),
            );
        }
        let mut pane = div()
            .h(px(self.dock_height))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .id("compose-dock-resize")
                    .h(px(6.0))
                    .cursor(CursorStyle::ResizeUpDown)
                    .hover(|style| style.bg(rgb(ACCENT)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.resize_origin =
                                Some((f32::from(event.position.y), this.dock_height));
                            cx.notify();
                        }),
                    ),
            )
            .child(toolbar);
        if let Some(form) = &self.compose_form {
            let mut controls = div()
                .flex()
                .gap_2()
                .px_4()
                .py_2()
                .child(
                    Self::button(
                        "compose-save",
                        if form.saving {
                            "Saving…"
                        } else {
                            "Save association"
                        },
                        true,
                        form.saving || self.connection.is_none(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.save_project(cx))),
                )
                .child(
                    Self::button("compose-cancel-config", "Cancel", false, form.saving).on_click(
                        cx.listener(|this, _, window, cx| {
                            if !this.compose_form.as_ref().is_some_and(|form| form.saving) {
                                this.compose_form = None;
                                this.focus.focus(window, cx);
                                cx.notify();
                            }
                        }),
                    ),
                );
            if let Some(error) = &form.error {
                controls = controls.child(
                    div()
                        .text_color(rgb(ERROR))
                        .text_size(px(12.0))
                        .child(error.clone()),
                );
            }
            pane = pane.child(controls);
            let mut fields = div().id("compose-form").flex_1().min_h_0().overflow_y_scroll().p_4().flex().flex_col().gap_2()
                .track_focus(&self.compose_focus).on_key_down(cx.listener(Self::compose_key))
                .child(div().text_size(px(11.0)).text_color(rgb(MUTED)).child("Review label suggestions. Tab: next field · Ctrl+A: replace field · Ctrl+V: paste · Escape: cancel"));
            for (index, label) in [
                "Project directory (absolute path)",
                "Compose files (one per line, in override order)",
                "Environment files (one per line; empty uses Compose's default .env)",
                "Profiles (one per line; empty means no explicit profiles)",
            ]
            .into_iter()
            .enumerate()
            {
                fields = fields.child(div().text_size(px(12.0)).child(label)).child(
                    div()
                        .id(SharedString::from(format!("compose-field-{index}")))
                        .debug_selector(move || format!("compose-field-{index}"))
                        .border_1()
                        .border_color(rgb(if form.active == index { ACCENT } else { BORDER }))
                        .rounded_md()
                        .px_3()
                        .py_2()
                        .text_size(px(12.0))
                        .font_family("DejaVu Sans Mono")
                        .cursor(CursorStyle::IBeam)
                        .child(format!(
                            "{}{}",
                            form.fields[index],
                            if form.active == index { "▏" } else { "" }
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(form) = &mut this.compose_form {
                                form.active = index;
                            }
                            this.compose_focus.focus(window, cx);
                            cx.notify();
                        })),
                );
            }
            return pane.child(fields);
        }
        pane = pane.child(
            div()
                .flex()
                .gap_2()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(
                    Self::button(
                        "project-tab-logs",
                        "Logs",
                        self.project_tab == ProjectTab::Logs,
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_tab = ProjectTab::Logs;
                        cx.notify();
                    })),
                )
                .child(
                    Self::button(
                        "project-tab-operations",
                        "Operations",
                        self.project_tab == ProjectTab::Operations,
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_tab = ProjectTab::Operations;
                        cx.notify();
                    })),
                ),
        );
        if self.project_tab == ProjectTab::Logs {
            return pane.child(self.project_logs_view(name, cx));
        }
        let mut body = div()
            .id("compose-output")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(operation) = self.compose_operations.get(name) {
            let status = if operation.running {
                format!("{} in progress…", operation.action.label())
            } else {
                match &operation.result {
                    Some(Ok(())) => format!("{} completed", operation.action.label()),
                    Some(Err(error)) => error.clone(),
                    None => "Operation ended".into(),
                }
            };
            body = body
                .child(
                    div()
                        .text_color(rgb(
                            if operation.result.as_ref().is_some_and(Result::is_err) {
                                ERROR
                            } else {
                                ACCENT
                            },
                        ))
                        .child(status),
                )
                .child(
                    div()
                        .font_family("DejaVu Sans Mono")
                        .text_size(px(12.0))
                        .child(
                            if operation.output.lines.is_empty()
                                && operation.output.partial.is_empty()
                            {
                                if operation.running {
                                    "Waiting for Compose output…"
                                } else {
                                    "Compose produced no output."
                                }
                                .into()
                            } else {
                                operation.output.text()
                            },
                        ),
                );
        } else if let Some(config) = project.and_then(|project| project.config.as_ref()) {
            body = body.child(div().text_size(px(12.0)).text_color(rgb(MUTED)).child(format!("Directory: {}\nFiles: {}\nEnvironment files: {}\nProfiles: {}\n\nStart creates/starts services without building images. Stop and Restart operate on project services. Volumes are preserved.",
                config.directory.display(), config.files.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(" → "),
                if config.env_files.is_empty() { "default .env".into() } else { config.env_files.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(" → ") },
                if config.profiles.is_empty() { "default services".into() } else { config.profiles.join(", ") })));
        } else {
            body = body.child(div().text_color(rgb(MUTED)).child("Associate Compose files and review environment files and profiles before managing this project. Individual container controls remain available."));
        }
        pane.child(body)
    }

    fn project_logs_view(&self, name: &str, cx: &mut Context<Self>) -> Div {
        #[cfg(all(test, feature = "headless-tests"))]
        self.project_rows_rendered.set(0);
        let mut controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                Self::button(
                    "project-log-follow",
                    if self.project_log_follow {
                        "Following"
                    } else {
                        "Follow paused"
                    },
                    self.project_log_follow,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_log_follow = !this.project_log_follow;
                    if this.project_log_follow {
                        this.project_log_scroll.scroll_to_bottom();
                    }
                    cx.notify();
                })),
            )
            .child(
                Self::button("project-log-copy", "Copy logs", false, false).on_click(cx.listener(
                    |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            this.project_logs.text(this.project_service.as_deref()),
                        ));
                        this.message = Some(("Retained project logs copied".into(), false));
                        cx.notify();
                    },
                )),
            )
            .child(
                Self::button(
                    "project-log-refresh",
                    "Refresh logs",
                    false,
                    self.connection.is_none(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.connection.is_some()
                        && let Some(project) = this.selected_project.clone()
                    {
                        this.begin_project_logs(project);
                        cx.notify();
                    }
                })),
            )
            .child(
                Self::button(
                    "project-service-all",
                    "All services",
                    self.project_service.is_none(),
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_service = None;
                    if this.project_log_follow {
                        this.project_log_scroll.scroll_to_bottom();
                    }
                    cx.notify();
                })),
            );
        let services = self
            .containers
            .iter()
            .filter(|container| container.project.as_deref() == Some(name))
            .filter_map(|container| container.service.clone())
            .chain(self.project_logs.services().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        for service in services {
            controls = controls.child(
                Self::button(
                    format!("project-service-{service}"),
                    service.clone(),
                    self.project_service.as_ref() == Some(&service),
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project_service = Some(service.clone());
                    if this.project_log_follow {
                        this.project_log_scroll.scroll_to_bottom();
                    }
                    cx.notify();
                })),
            );
        }
        let indices = self
            .project_logs
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                self.project_service
                    .as_ref()
                    .is_none_or(|service| line.service == *service)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let widest = indices
            .iter()
            .enumerate()
            .max_by_key(|(_, index)| {
                let line = &self.project_logs.lines[**index];
                line.text.len() + line.service.len() + line.name.len()
            })
            .map(|(index, _)| index);
        let lines = uniform_list(
            "project-log-scroll",
            indices.len().max(1),
            cx.processor(move |this, range: std::ops::Range<usize>, _, _| {
                #[cfg(all(test, feature = "headless-tests"))]
                this.project_rows_rendered
                    .set(this.project_rows_rendered.get() + range.len());
                range
                    .map(|item| {
                        if let Some(index) = indices.get(item)
                            && let Some(line) = this.project_logs.lines.get(*index)
                        {
                            let index = *index;
                            div()
                                .h(px(18.0))
                                .whitespace_nowrap()
                                .text_color(rgb(if line.error { ERROR } else { TEXT }))
                                .debug_selector(move || format!("project-log-line-{index}"))
                                .child(line.display())
                        } else {
                            div()
                                .h(px(18.0))
                                .text_color(rgb(MUTED))
                                .child("Waiting for log output from this project's services…")
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .with_width_from_item(widest)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(&self.project_log_scroll)
        .flex_1()
        .min_h_0()
        .p_3()
        .font_family("DejaVu Sans Mono")
        .text_size(px(12.0));
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(0x101819))
            .child(controls)
            .child(lines)
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child(self.project_log_status.clone()),
            )
    }
}

impl Render for FerryDock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self
            .containers
            .iter()
            .filter(|container| container.running())
            .count();
        let connected = self.connection.is_some();
        let connection = self
            .connection
            .as_ref()
            .map(|connection| format!("● {}", connection.name))
            .unwrap_or_else(|| "● Connecting to Docker…".into());
        let endpoint = self
            .connection
            .as_ref()
            .map(|connection| connection.endpoint.clone())
            .unwrap_or_default();
        let height = f32::from(window.viewport_size().height);
        self.dock_height = self.dock_height.min((height - 220.0).max(200.0));
        let mut sidebar = div()
            .w(px(155.0))
            .flex_shrink_0()
            .bg(rgb(PANEL))
            .border_r_1()
            .border_color(rgb(BORDER))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .px_2()
                    .py_2()
                    .child("WORKSPACE"),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(SELECTED))
                    .text_color(rgb(ACCENT))
                    .child("Containers"),
            )
            .child(div().flex_1());
        sidebar = sidebar
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "{running} running / {} total",
                        self.containers.len()
                    )),
            )
            .child(div().text_size(px(11.0)).text_color(rgb(MUTED)).child(
                "Ctrl+F  Search\nCtrl+R  Refresh\nCtrl+1/2/3  Details\nCtrl+4  Project logs",
            ));
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(19.0))
                    .child("FerryDock"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(if connected { ACCENT } else { MUTED }))
                    .child(connection),
            )
            .child(
                Self::button("refresh", "Refresh", false, false)
                    .on_click(cx.listener(|this, _, _, _| this.backend.send(Command::Refresh))),
            );
        let search = div()
            .id("search")
            .track_focus(&self.search_focus)
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .px_3()
            .py_1()
            .w(px(240.0))
            .text_size(px(12.0))
            .text_color(rgb(if self.filter.is_empty() { MUTED } else { TEXT }))
            .cursor(CursorStyle::IBeam)
            .on_click(cx.listener(|this, _, window, cx| this.search_focus.focus(window, cx)))
            .on_key_down(cx.listener(Self::search_key))
            .child(if self.filter.is_empty() {
                "Search containers…  Ctrl+F".into()
            } else {
                format!("{}▏", self.filter)
            });
        let mut main = div().flex_1().min_w_0().h_full().flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_5()
                .py_3()
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(div().font_weight(FontWeight::MEDIUM).child("Containers"))
                .child(div().flex_1())
                .child(search)
                .child(
                    Self::button(
                        "all",
                        if self.running_only {
                            "Running only"
                        } else {
                            "All containers"
                        },
                        self.running_only,
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.running_only = !this.running_only;
                        cx.notify();
                    })),
                ),
        );
        if let Some(error) = &self.connection_error {
            main = main.child(
                div()
                    .px_4()
                    .py_2()
                    .bg(rgb(0x38262a))
                    .text_size(px(12.0))
                    .text_color(rgb(ERROR))
                    .child(format!("{error} · retrying every 5 seconds")),
            );
        }
        if let Some((message, error)) = &self.message {
            main =
                main.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .py_2()
                        .text_size(px(12.0))
                        .text_color(rgb(if *error { ERROR } else { ACCENT }))
                        .child(div().flex_1().child(message.clone()))
                        .child(Self::button("dismiss", "Dismiss", false, false).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.message = None;
                                cx.notify();
                            }),
                        )),
                );
        }
        main = main.child(self.resources(cx)).child(self.detail(cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .font_family("Inter")
            .text_size(px(14.0))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::root_key))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if let Some((origin, height)) = this.resize_origin {
                    this.dock_height = (height + origin - f32::from(event.position.y))
                        .clamp(200.0, (height + 400.0).max(200.0));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.resize_origin = None;
                    cx.notify();
                }),
            )
            .child(header)
            .child(div().flex_1().min_h_0().flex().child(sidebar).child(main))
            .child(
                div()
                    .px_4()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(10.0))
                    .text_color(rgb(MUTED))
                    .child(endpoint),
            )
    }
}

#[cfg(all(test, feature = "headless-tests", target_os = "linux"))]
#[path = "headless_tests.rs"]
mod headless_tests;
