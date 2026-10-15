//! Embedded terminal session state, controls, and view routing.
use super::FerryDock;
use super::Tab;
use super::ui::empty_state;
use super::ui::palette;
use crate::docker::Command;
use crate::docker::Shell;
use crate::model::Container;
use crate::terminal::TerminalView;
use gpui::{prelude::*, *};
use gpui_kit::component::{
    radio::{Radio, RadioGroup},
    status_bar::StatusBar,
    toolbar::Toolbar,
};

pub(super) struct Terminal {
    pub(super) generation: u64,
    pub(super) view: Option<Entity<TerminalView>>,
    pub(super) shell: Shell,
}
impl Default for Terminal {
    fn default() -> Self {
        Self {
            generation: 0,
            view: None,
            shell: Shell::Auto,
        }
    }
}

impl FerryDock {
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
        self.terminal.generation += 1;
        let generation = self.terminal.generation;
        let commands = self.backend.commands.clone();
        let view = cx.new(|cx| TerminalView::new(generation, commands, cx));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        view.update(cx, |view, cx| view.focus(window, cx));
        self.terminal.view = Some(view);
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
                    this.backend.send(Command::CloseTerminal);
                    this.terminal.generation += 1;
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
