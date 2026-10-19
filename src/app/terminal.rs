//! Embedded terminal session state, controls, and view routing.
use super::FerryDock;
use super::Tab;
use super::ui::empty_state;
use super::ui::palette;
use crate::docker::Shell;
use crate::docker::{Command, Event};
use crate::model::Container;
use crate::terminal::TerminalView;
use gpui::{prelude::*, *};
use gpui_kit::component::{
    radio::{Radio, RadioGroup},
    status_bar::StatusBar,
    toolbar::Toolbar,
};
use std::collections::HashMap;

const SESSION_LIMIT: usize = 6;

struct Session {
    generation: u64,
    view: Entity<TerminalView>,
    ended: bool,
}

pub(super) struct Terminal {
    // Display aliases for the selected container; sessions own all live views.
    pub(super) generation: u64,
    pub(super) view: Option<Entity<TerminalView>>,
    pub(super) shell: Shell,
    next_generation: u64,
    sessions: HashMap<String, Session>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            generation: 0,
            view: None,
            shell: Shell::Auto,
            next_generation: 0,
            sessions: HashMap::new(),
        }
    }
}

impl Terminal {
    pub(super) fn select(&mut self, id: &str) {
        let session = self.sessions.get(id);
        self.view = session.map(|session| session.view.clone());
        self.generation = session.map_or(self.next_generation, |session| session.generation);
    }

    pub(super) fn live(&self) -> bool {
        self.sessions.values().any(|session| !session.ended)
    }

    pub(super) fn event_view(&self, generation: u64) -> Option<&Entity<TerminalView>> {
        self.sessions
            .values()
            .find(|session| session.generation == generation && !session.ended)
            .map(|session| &session.view)
    }
}

impl FerryDock {
    pub(super) fn close_container_terminal(
        &mut self,
        id: &str,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.terminal.sessions.get_mut(id) {
            if session.ended {
                return;
            }
            self.backend.send(Command::CloseTerminal {
                generation: session.generation,
            });
            session.ended = true;
            session.view.update(cx, |view, cx| {
                view.ready = false;
                view.at_prompt = false;
                view.status = status.to_owned();
                if let Some(favorite) = &mut view.favorite
                    && matches!(
                        favorite.status,
                        crate::terminal::FavoriteStatus::Starting
                            | crate::terminal::FavoriteStatus::Running
                    )
                {
                    favorite.status = crate::terminal::FavoriteStatus::NotSent(
                        "Session closed; command completion is unknown".into(),
                    );
                }
                cx.notify();
            });
        }
    }

    pub(super) fn close_all_container_terminals(&mut self, status: &str, cx: &mut Context<Self>) {
        for id in self.terminal.sessions.keys().cloned().collect::<Vec<_>>() {
            self.close_container_terminal(&id, status, cx);
        }
    }

    pub(super) fn reconcile_container_terminals(
        &mut self,
        containers: &[Container],
        cx: &mut Context<Self>,
    ) {
        for id in self.terminal.sessions.keys().cloned().collect::<Vec<_>>() {
            match containers.iter().find(|container| container.id == id) {
                Some(container) if container.running() => {}
                Some(_) => self.close_container_terminal(
                    &id,
                    "Container stopped · start it to open a new shell",
                    cx,
                ),
                None => {
                    self.close_container_terminal(&id, "Container removed · shell unavailable", cx);
                    self.terminal.sessions.remove(&id);
                }
            }
        }
    }

    pub(super) fn container_terminal_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let generation = match &event {
            Event::TerminalReady { generation }
            | Event::TerminalData { generation, .. }
            | Event::TerminalEnded { generation, .. }
            | Event::TerminalPrompt { generation, .. }
            | Event::FavoriteStarted { generation, .. }
            | Event::FavoriteFinished { generation, .. } => *generation,
            _ => return,
        };
        let Some(terminal) = self.terminal.event_view(generation).cloned() else {
            return;
        };
        match event {
            Event::TerminalReady { .. } => terminal.update(cx, |view, cx| {
                view.ready = true;
                view.status =
                    "Connected · Ctrl+Shift+C/V to copy/paste · Shift+PageUp/Down for scrollback"
                        .into();
                cx.notify();
            }),
            Event::TerminalData { data, .. } => {
                terminal.update(cx, |view, cx| view.feed(&data, cx))
            }
            Event::TerminalEnded { error, .. } => {
                terminal.update(cx, |view, cx| {
                    view.ready = false;
                    view.at_prompt = false;
                    view.status = error
                        .map(|error| format!("Shell failed: {error}"))
                        .unwrap_or_else(|| "Shell exited · reopen to start a new session".into());
                    cx.notify();
                });
                for session in self
                    .terminal
                    .sessions
                    .values_mut()
                    .filter(|s| s.generation == generation)
                {
                    session.ended = true;
                }
            }
            event => self.handle_favorite_terminal_event(event, cx),
        }
    }

    pub(super) fn open_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(container) = self.selected_container() else {
            return;
        };
        if !container.running()
            || self.connection.is_none()
            || self.containers.busy.contains_key(&container.id)
            || container
                .project
                .as_ref()
                .is_some_and(|project| self.project_busy(project))
        {
            return;
        }
        let id = container.id.clone();
        if self
            .terminal
            .sessions
            .get(&id)
            .is_some_and(|session| !session.ended)
        {
            self.switch_tab(Tab::Terminal, window, cx);
            self.expand_dock(580.0, window, cx);
            return;
        }
        if !self.terminal.sessions.contains_key(&id)
            && self.terminal.sessions.len() >= SESSION_LIMIT
        {
            if let Some(expired) = self
                .terminal
                .sessions
                .iter()
                .filter(|(_, s)| s.ended)
                .min_by_key(|(_, s)| s.generation)
                .map(|(id, _)| id.clone())
            {
                self.terminal.sessions.remove(&expired);
            } else {
                Self::notify(
                    "Disconnect a container shell before opening another (limit 6)",
                    true,
                    window,
                    cx,
                );
                return;
            }
        }
        if let Some(previous) = self.terminal.sessions.get(&id) {
            self.backend.send(Command::CloseTerminal {
                generation: previous.generation,
            });
        }
        self.terminal.next_generation += 1;
        let generation = self.terminal.next_generation;
        self.terminal.generation = generation;
        let commands = self.backend.commands.clone();
        let view = cx.new(|cx| TerminalView::new(generation, commands, cx));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        view.update(cx, |view, cx| view.focus(window, cx));
        self.terminal.view = Some(view.clone());
        self.terminal.sessions.insert(
            id.clone(),
            Session {
                generation,
                view,
                ended: false,
            },
        );
        self.backend.send(Command::Terminal {
            id,
            generation,
            shell: self.terminal.shell,
            columns: 80,
            rows: 24,
        });
        self.tab = Tab::Terminal;
        self.expand_dock(580.0, window, cx);
        cx.notify();
    }

    pub(super) fn terminal_view(&self, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let running =
            self.selected_container().is_some_and(Container::running) && self.connection.is_some();
        let ready = self
            .terminal
            .view
            .as_ref()
            .is_some_and(|view| view.read(cx).ready);
        let mut toolbar = Toolbar::new("terminal-toolbar")
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(colors.border));
        toolbar = toolbar.child(
            Self::button(
                "open-shell",
                if self.terminal.view.is_none() {
                    "Open shell"
                } else if ready {
                    "Connected"
                } else {
                    "Reopen shell"
                },
                ready,
                !running || ready,
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if !this
                    .terminal
                    .view
                    .as_ref()
                    .is_some_and(|view| view.read(cx).ready)
                {
                    this.open_terminal(window, cx);
                }
            })),
        );
        let shells = [Shell::Auto, Shell::Bash, Shell::Sh];
        toolbar = toolbar.content(
            RadioGroup::horizontal("shell-choice")
                .flex_none()
                .w_auto()
                .selected_index(shells.iter().position(|shell| {
                    std::mem::discriminant(shell) == std::mem::discriminant(&self.terminal.shell)
                }))
                .disabled(ready)
                .children(shells.map(|shell| {
                    Radio::new(shell.label())
                        .label(shell.label())
                        .when(matches!(shell, Shell::Auto), |radio| {
                            radio.tooltip("Try Bash, then fall back to sh")
                        })
                }))
                .on_change(cx.listener(move |this, index: &usize, _, cx| {
                    if !this
                        .terminal
                        .view
                        .as_ref()
                        .is_some_and(|view| view.read(cx).ready)
                    {
                        this.terminal.shell = shells[*index];
                        cx.notify();
                    }
                })),
        );
        toolbar = toolbar
            .content(div().flex_1())
            .child(
                Self::button("paste-terminal", "Paste", false, !ready, cx).on_click(cx.listener(
                    |this, _, window, cx| {
                        if let Some(view) = &this.terminal.view {
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
                    self.terminal.view.is_none(),
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(id) = this.containers.selected.clone() {
                        this.close_container_terminal(&id, "Disconnected", cx);
                        this.terminal.sessions.remove(&id);
                    }
                    this.terminal.view = None;
                    cx.notify();
                })),
            );
        let mut pane = div()
            .flex()
            .flex_col()
            .size_full()
            .child(toolbar)
            .child(self.terminal_favorites_toolbar(cx));
        if let Some(terminal) = &self.terminal.view {
            let status = terminal.read(cx).status.clone();
            pane = pane
                .child(div().flex_1().min_h_0().child(terminal.clone()))
                .child(StatusBar::new().left(status));
        } else {
            pane = pane.child(empty_state(
                "Terminal",
                if running {
                    "Open a shell inside this container"
                } else {
                    "Start this container to open a shell"
                },
            ));
        }
        pane
    }
}
