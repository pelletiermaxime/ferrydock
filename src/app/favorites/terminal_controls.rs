//! Favorites are sent to the selected container's existing interactive shell.
use super::*;
use crate::{
    docker::{Command, Event},
    terminal::{FavoriteRun, FavoriteStatus},
};
use gpui_kit::{
    base::TestSupportExt,
    component::{
        Sizable,
        select::{Select, SelectItem},
        toolbar::Toolbar,
    },
};

#[derive(Clone)]
pub(super) struct Choice {
    id: u64,
    label: SharedString,
    preview: SharedString,
}
impl SelectItem for Choice {
    type Value = u64;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &u64 {
        &self.id
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(self.label.clone())
            .child(
                div()
                    .text_sm()
                    .opacity(0.65)
                    .text_ellipsis()
                    .child(self.preview.clone()),
            )
    }
}

impl FerryDock {
    pub(super) fn sync_terminal_favorites(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = self
            .connection
            .as_ref()
            .zip(self.selected_container())
            .and_then(|(connection, container)| {
                let target = match (&container.project, &container.service) {
                    (Some(project), Some(service)) => Target::Service {
                        project: project.clone(),
                        service: service.clone(),
                    },
                    (None, _) => Target::Container {
                        name: container.name.clone(),
                    },
                    _ => return None,
                };
                Some((
                    connection.endpoint.clone(),
                    target,
                    std::sync::Arc::as_ptr(&self.favorites.items) as usize,
                ))
            });
        if self.favorites.terminal_picker.is_some() && key == self.favorites.terminal_key {
            return false;
        }
        let choices = key
            .as_ref()
            .map(|(endpoint, target, _)| {
                self.favorites
                    .items
                    .iter()
                    .filter(|f| &f.endpoint == endpoint && &f.target == target)
                    .map(|f| Choice {
                        id: f.id,
                        label: f.label.clone().into(),
                        preview: f
                            .command
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .take(120)
                            .collect::<String>()
                            .into(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(picker) = &self.favorites.terminal_picker {
            let same_target = key
                .as_ref()
                .zip(self.favorites.terminal_key.as_ref())
                .is_some_and(|(a, b)| a.0 == b.0 && a.1 == b.1);
            let selected = same_target
                .then(|| picker.read(cx).selected_value().copied())
                .flatten();
            let index = choices
                .iter()
                .position(|choice| Some(choice.id) == selected)
                .map(IndexPath::new);
            picker.update(cx, |picker, cx| {
                picker.set_items(SearchableVec::new(choices), window, cx);
                picker.set_selected_index(index, window, cx);
            });
        } else {
            let picker = cx.new(|cx| {
                SelectState::new(SearchableVec::new(choices), None, window, cx).searchable(true)
            });
            cx.subscribe(
                &picker,
                |_, _, _: &SelectEvent<SearchableVec<Choice>>, cx| cx.notify(),
            )
            .detach();
            self.favorites.terminal_picker = Some(picker);
        }
        self.favorites.terminal_key = key;
        true
    }

    fn selected_terminal_favorite(&self, cx: &App) -> Option<&Favorite> {
        let id = self
            .favorites
            .terminal_picker
            .as_ref()?
            .read(cx)
            .selected_value()?;
        self.favorites.items.iter().find(|f| f.id == *id)
    }

    fn terminal_favorite_available(&self, cx: &App) -> Result<(), &'static str> {
        let container = self
            .selected_container()
            .ok_or("Select a container to run a favorite")?;
        let connection = self.connection.as_ref().ok_or("Docker is disconnected")?;
        if !container.running() {
            return Err("Start this container before running a favorite");
        }
        if self.favorite_operation_conflict(container) {
            return Err("Wait for the Docker operation to finish");
        }
        let view = self
            .terminal
            .view
            .as_ref()
            .ok_or("Open a shell to run a favorite")?
            .read(cx);
        if !view.ready {
            return Err("Open a shell to run a favorite");
        }
        if view
            .favorite
            .as_ref()
            .is_some_and(|f| matches!(f.status, FavoriteStatus::Starting | FavoriteStatus::Running))
        {
            return Err("Wait for the current favorite to finish");
        }
        if !view.at_prompt {
            return Err(if view.input_count == 0 {
                "Waiting for the shell to initialize"
            } else {
                "Finish or clear the command being typed in the terminal"
            });
        }
        let favorite = self
            .selected_terminal_favorite(cx)
            .ok_or("Choose a favorite command")?;
        if favorite.endpoint != connection.endpoint || !favorite.target.matches(container) {
            return Err("Choose a favorite for this container and Docker endpoint");
        }
        Ok(())
    }

    fn run_terminal_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_favorite_available(cx).is_err() {
            return;
        }
        let favorite = self.selected_terminal_favorite(cx).unwrap().clone();
        self.favorites.used_pending.insert(favorite.id);
        let id = self.selected_container().unwrap().id.clone();
        self.favorites.next_run += 1;
        let run = self.favorites.next_run;
        let generation = self.terminal.generation;
        if let Some(terminal) = &self.terminal.view {
            terminal.update(cx, |view, cx| {
                view.favorite = Some(FavoriteRun {
                    id: run,
                    label: favorite.label.clone(),
                    status: FavoriteStatus::Starting,
                });
                view.focus(window, cx);
                cx.notify();
            });
        }
        self.backend.send(Command::RunFavorite {
            id,
            generation,
            run,
            endpoint: favorite.endpoint,
            target: favorite.target,
            command: favorite.command,
        });
        cx.notify();
    }

    pub(in crate::app) fn handle_favorite_terminal_event(
        &mut self,
        event: Event,
        cx: &mut Context<Self>,
    ) {
        let generation = match &event {
            Event::TerminalPrompt { generation, .. }
            | Event::FavoriteStarted { generation, .. }
            | Event::FavoriteFinished { generation, .. } => *generation,
            _ => return,
        };
        let Some(terminal) = self.terminal.event_view(generation) else {
            return;
        };
        terminal.update(cx, |view, cx| {
            match event {
                Event::TerminalPrompt {
                    input_count,
                    at_prompt,
                    ..
                } if input_count == view.input_count => view.at_prompt = at_prompt,
                Event::FavoriteStarted { run, .. } => {
                    if let Some(favorite) = &mut view.favorite
                        && favorite.id == run
                        && matches!(favorite.status, FavoriteStatus::Starting)
                    {
                        favorite.status = FavoriteStatus::Running;
                    }
                }
                Event::FavoriteFinished { run, result, .. } => {
                    if let Some(favorite) = &mut view.favorite
                        && favorite.id == run
                    {
                        favorite.status = match result {
                            Ok(code) => FavoriteStatus::Completed(code),
                            Err(error) => FavoriteStatus::NotSent(error),
                        };
                    }
                }
                _ => return,
            }
            cx.notify();
        });
    }

    pub(in crate::app) fn terminal_favorites_toolbar(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = crate::app::ui::palette(cx);
        let available = self.terminal_favorite_available(cx);
        let mut status = available
            .err()
            .unwrap_or("Ready at shell prompt")
            .to_owned();
        let mut color = colors.muted;
        if let Some(favorite) = self
            .terminal
            .view
            .as_ref()
            .and_then(|view| view.read(cx).favorite.as_ref())
        {
            match &favorite.status {
                FavoriteStatus::Starting => status = format!("Sending · {}", favorite.label),
                FavoriteStatus::Running => status = format!("Running · {}", favorite.label),
                FavoriteStatus::Completed(0) => {
                    status = format!("Completed · {} · exit 0", favorite.label);
                    color = colors.accent;
                }
                FavoriteStatus::Completed(code) => {
                    status = format!("Failed · {} · exit {code}", favorite.label);
                    color = colors.error;
                }
                FavoriteStatus::NotSent(error) => {
                    status = error.clone();
                    color = colors.error;
                }
            }
        }
        let detail = status.clone();
        Toolbar::new("terminal-favorites-toolbar")
            .h_auto()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(colors.border))
            .when_some(
                self.favorites.terminal_picker.as_ref(),
                |toolbar, picker| {
                    toolbar.child(
                        Select::new(picker)
                            .id("terminal-favorite-picker")
                            .small()
                            .w(px(240.0))
                            .placeholder("Favorite command…")
                            .accessibility_label("Favorite command for this container"),
                    )
                },
            )
            .child(
                Self::button(
                    "terminal-favorite-run",
                    "Run",
                    false,
                    available.is_err(),
                    cx,
                )
                .tooltip(
                    available
                        .err()
                        .unwrap_or("Run this favorite in the current shell"),
                )
                .on_click(
                    cx.listener(|this, _, window, cx| this.run_terminal_favorite(window, cx)),
                ),
            )
            .child(
                Self::button(
                    "terminal-favorite-manage",
                    "Manage…",
                    false,
                    self.connection.is_none(),
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.container_favorites(window, cx))),
            )
            .content(
                div()
                    .id("terminal-favorite-status")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(rgb(color))
                    .text_ellipsis()
                    .child(status)
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(detail.clone()).build(window, cx)
                    }),
            )
    }
}
