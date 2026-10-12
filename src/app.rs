use std::{collections::HashMap, time::Duration};

use gpui::{prelude::*, *};

use crate::{
    docker::{Action, Backend, Command, Connection, Event, Shell},
    model::{Container, LogBuffer, groups},
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

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Logs,
    Terminal,
    Inspect,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Self::Logs => "Logs",
            Self::Terminal => "Terminal",
            Self::Inspect => "Inspect",
        }
    }
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
    log_scroll: ScrollHandle,
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
                        for _ in 0..64 {
                            match this.events.try_recv() {
                                Ok(event) => {
                                    this.handle_event(event, cx);
                                    changed = true;
                                }
                                Err(_) => break,
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
            log_scroll: ScrollHandle::new(),
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
        }
    }

    fn selected_container(&self) -> Option<&Container> {
        self.selected
            .as_ref()
            .and_then(|id| self.containers.iter().find(|container| &container.id == id))
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&id) {
            return;
        }
        self.backend.send(Command::CloseTerminal);
        self.terminal = None;
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
            Event::Connected(connection) => {
                self.connection = Some(connection);
                self.connection_error = None;
                self.message = None;
                self.reopen_logs = true;
            }
            Event::Disconnected(error) => {
                self.connection = None;
                self.connection_error = Some(error);
                self.busy.clear();
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
                    if let Some(id) = next {
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
        if self.connection.is_none() || self.busy.contains_key(&container.id) {
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
            "enter" => self.open_terminal(window, cx),
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
        id: &'static str,
        text: impl Into<SharedString>,
        active: bool,
        disabled: bool,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_owned())
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

    fn resources(&self, cx: &mut Context<Self>) -> Div {
        let groups = groups(&self.containers, self.running_only, &self.filter);
        let mut list = div().flex().flex_col().gap_1().p_3();
        if groups.is_empty() {
            list = list.child(div().p_6().text_color(rgb(MUTED)).child(
                if self.connection.is_none() {
                    "Waiting for Docker…"
                } else if !self.filter.is_empty() {
                    "No containers match your search"
                } else if self.running_only {
                    "No running containers · switch to All to see stopped containers"
                } else {
                    "No containers on this Docker context"
                },
            ));
        }
        for (project, containers) in groups {
            let count = containers.len();
            let mut group = div().flex().flex_col().mb_2().child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_2()
                    .text_size(px(12.0))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(project.label().to_owned()),
                    )
                    .child(div().text_color(rgb(MUTED)).child(format!(
                        "{count} container{}",
                        if count == 1 { "" } else { "s" }
                    ))),
            );
            for container in containers {
                let selected = self.selected.as_ref() == Some(&container.id);
                let running = container.running();
                let id = container.id.clone();
                let busy = self.busy.get(&container.id).cloned();
                group = group.child(
                    div()
                        .id(SharedString::from(format!("row-{}", container.id)))
                        .debug_selector(|| format!("row-{}", container.id))
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(if selected { SELECTED } else { BG }))
                        .hover(|style| style.bg(rgb(PANEL)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select(id.clone(), cx);
                            this.focus.focus(window, cx);
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(px(13.0)).child(container.name))
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(MUTED))
                                        .text_ellipsis()
                                        .child(container.image),
                                ),
                        )
                        .child(
                            div()
                                .w(px(160.0))
                                .text_size(px(12.0))
                                .text_color(rgb(if running { ACCENT } else { MUTED }))
                                .child(busy.unwrap_or_else(|| format!("● {}", container.state)))
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(MUTED))
                                        .text_ellipsis()
                                        .child(container.status),
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
                                    container.ports
                                }),
                        ),
                );
            }
            list = list.child(group);
        }
        list
    }

    fn logs_view(&self, cx: &mut Context<Self>) -> Div {
        let mut lines = div()
            .id("log-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .overflow_x_scroll()
            .track_scroll(&self.log_scroll)
            .p_3()
            .font_family("DejaVu Sans Mono")
            .text_size(px(12.0));
        for (index, line) in self.logs.lines.iter().enumerate() {
            lines = lines.child(
                div()
                    .id(index)
                    .min_h(px(18.0))
                    .whitespace_nowrap()
                    .child(line.clone()),
            );
        }
        if !self.logs.partial.is_empty() {
            lines = lines.child(div().min_h(px(18.0)).child(self.logs.partial.clone()));
        }
        if self.logs.lines.is_empty() && self.logs.partial.is_empty() {
            lines = lines.child(
                div()
                    .text_color(rgb(MUTED))
                    .child("Waiting for log output…"),
            );
        }
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
        let Some(container) = self.selected_container() else {
            return div()
                .h(px(self.dock_height))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(MUTED))
                .child("Select a container to view logs, open a shell, or inspect it");
        };
        let unavailable = self.connection.is_none() || self.busy.contains_key(&container.id);
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
            }),
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
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child("Ctrl+F  Search\nCtrl+R  Refresh\nCtrl+1/2/3  Details"),
            );
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
        let mut main = div().flex_1().min_w_0().flex().flex_col().child(
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
        main = main
            .child(
                div()
                    .id("containers-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.resources(cx)),
            )
            .child(self.detail(cx));
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
