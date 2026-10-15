//! Bounded independent terminals for global and pinned favorite launches.
use super::*;
use crate::{
    docker::{Command, Event},
    terminal::{FavoriteRun, FavoriteStatus, TerminalView},
};
use gpui_kit::component::list::ListState;
use std::time::{Duration, Instant};
mod console;
mod launcher;
const MAX_SESSIONS: usize = 6;

#[derive(Default)]
pub(super) struct QuickRun {
    launcher: Option<Entity<ListState<launcher::Launcher>>>,
    console_open: bool,
    title: String,
    error: Option<String>,
    return_focus: Option<FocusHandle>,
    sessions: Vec<Session>,
    active: Option<u64>,
    sequence: u64,
}
struct Session {
    generation: u64,
    container: String,
    name: String,
    endpoint: String,
    target: Target,
    view: Entity<TerminalView>,
    pending: Option<(Favorite, Instant)>,
}
impl FerryDock {
    pub(in crate::app) fn quick_surface_open(&self) -> bool {
        self.favorites.quick.console_open || self.favorites.quick.launcher.is_some()
    }
    pub(in crate::app) fn quick_run_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.control && event.keystroke.key == "k" {
            if !window.has_active_dialog(cx)
                || self.favorites.quick.console_open
                || self.favorites.quick.launcher.is_some()
            {
                self.open_quick_launcher(window, cx);
            }
            cx.stop_propagation();
        }
    }
    fn quick_available(&self, favorite: &Favorite, id: &str) -> Result<(), String> {
        if !self
            .connection
            .as_ref()
            .is_some_and(|c| c.endpoint == favorite.endpoint)
        {
            return Err("This favorite belongs to another Docker endpoint".into());
        }
        let c = self
            .containers
            .items
            .iter()
            .find(|c| c.id == id && favorite.target.matches(c))
            .ok_or("This container is no longer available; choose the favorite again")?;
        if !c.running() {
            return Err("Start this container before running the favorite".into());
        }
        if self.containers.busy.contains_key(id)
            || c.project.as_ref().is_some_and(|p| self.project_busy(p))
            || self.volumes.mutating()
            || self.images.pruning.is_some()
        {
            return Err("Wait for the Docker operation to finish".into());
        }
        Ok(())
    }
    fn choose_quick_favorite(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(favorite) = self.favorites.items.iter().find(|f| f.id == id).cloned() else {
            return;
        };
        let mut targets = self
            .containers
            .items
            .iter()
            .filter(|c| c.running() && favorite.target.matches(c))
            .collect::<Vec<_>>();
        targets.sort_by(|a, b| a.name.cmp(&b.name));
        if targets.len() == 1 {
            let container = targets[0].id.clone();
            self.start_quick_favorite(id, container, window, cx);
        } else if targets.is_empty() {
            self.favorites.quick.error = Some(format!(
                "No running container for {}. Start the service, then try again.",
                favorite.target.label()
            ));
            cx.notify();
        } else {
            let rows = targets
                .into_iter()
                .map(|c| launcher::Row {
                    title: c.name.clone(),
                    detail: format!(
                        "{} · {}",
                        favorite.label,
                        favorite.command.replace('\n', " · ")
                    ),
                    search: format!("{} {}", c.name, c.id).to_lowercase(),
                    action: launcher::Launch::Replica(id, c.id.clone()),
                })
                .collect();
            self.show_quick_launcher(
                format!("Choose instance · {}", favorite.target.label()),
                rows,
                window,
                cx,
            );
        }
    }
    fn start_quick_favorite(
        &mut self,
        id: u64,
        container: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(favorite) = self.favorites.items.iter().find(|f| f.id == id).cloned() else {
            return;
        };
        if let Err(error) = self.quick_available(&favorite, &container) {
            self.favorites.quick.error = Some(error);
            cx.notify();
            return;
        }
        let reusable = self.favorites.quick.sessions.iter().position(|s| {
            let v = s.view.read(cx);
            s.container == container
                && s.endpoint == favorite.endpoint
                && s.pending.is_none()
                && v.ready
                && v.at_prompt
                && !v.favorite.as_ref().is_some_and(|f| {
                    matches!(f.status, FavoriteStatus::Starting | FavoriteStatus::Running)
                })
        });
        let index = if let Some(index) = reusable {
            index
        } else {
            if self.favorites.quick.sessions.len() >= MAX_SESSIONS {
                self.favorites.quick.error = Some("Six quick-run shells are open. Open Quick runs and end a session before starting another.".into());
                cx.notify();
                return;
            }
            self.favorites.quick.sequence += 1;
            let generation = (1_u64 << 63) | self.favorites.quick.sequence;
            let commands = self.backend.commands.clone();
            let view = cx.new(|cx| TerminalView::new(generation, commands, cx));
            cx.observe(&view, |_, _, cx| cx.notify()).detach();
            let name = self
                .containers
                .items
                .iter()
                .find(|c| c.id == container)
                .unwrap()
                .name
                .clone();
            self.favorites.quick.sessions.push(Session {
                generation,
                container: container.clone(),
                name,
                endpoint: favorite.endpoint.clone(),
                target: favorite.target.clone(),
                view,
                pending: None,
            });
            self.backend.send(Command::QuickTerminal {
                id: container,
                generation,
            });
            self.favorites.quick.sessions.len() - 1
        };
        let session = &mut self.favorites.quick.sessions[index];
        session.pending = Some((favorite, Instant::now() + Duration::from_secs(15)));
        self.favorites.quick.active = Some(session.generation);
        self.favorites.quick.error = None;
        self.open_quick_console(window, cx);
        self.poll_quick_runs(cx);
        cx.notify();
    }
    pub(in crate::app::favorites) fn poll_quick_runs(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for index in 0..self.favorites.quick.sessions.len() {
            let s = &self.favorites.quick.sessions[index];
            let Some((favorite, deadline)) = &s.pending else {
                continue;
            };
            let available = self.quick_available(favorite, &s.container);
            let ready = s.view.read(cx).ready && s.view.read(cx).at_prompt;
            let failure = available.err().or_else(|| {
                (Instant::now() > *deadline).then(|| {
                    "Shell did not become ready within 15 seconds; command was not sent".into()
                })
            });
            if failure.is_none() && !ready {
                continue;
            }
            let s = &mut self.favorites.quick.sessions[index];
            let (favorite, _) = s.pending.take().unwrap();
            self.favorites.next_run += 1;
            let run = self.favorites.next_run;
            s.view.update(cx, |v, cx| {
                v.favorite = Some(FavoriteRun {
                    id: run,
                    label: favorite.label.clone(),
                    status: failure
                        .clone()
                        .map_or(FavoriteStatus::Starting, FavoriteStatus::NotSent),
                });
                cx.notify();
            });
            if failure.is_none() {
                self.favorites.used_pending.insert(favorite.id);
                self.backend.send(Command::RunFavorite {
                    id: s.container.clone(),
                    generation: s.generation,
                    run,
                    endpoint: favorite.endpoint,
                    target: favorite.target,
                    command: favorite.command,
                });
            }
            changed = true;
        }
        changed
    }
    pub(in crate::app) fn quick_terminal_event(
        &mut self,
        event: &Event,
        cx: &mut Context<Self>,
    ) -> bool {
        let generation = match event {
            Event::TerminalReady { generation }
            | Event::TerminalData { generation, .. }
            | Event::TerminalPrompt { generation, .. }
            | Event::TerminalEnded { generation, .. }
            | Event::FavoriteStarted { generation, .. }
            | Event::FavoriteFinished { generation, .. } => *generation,
            Event::Disconnected(_) => {
                for s in &mut self.favorites.quick.sessions {
                    s.pending = None;
                    s.view.update(cx, |v, cx| {
                        v.ready = false;
                        v.at_prompt = false;
                        v.status = "Docker disconnected · command session ended".into();
                        if let Some(f) = &mut v.favorite
                            && matches!(
                                f.status,
                                FavoriteStatus::Starting | FavoriteStatus::Running
                            )
                        {
                            f.status = FavoriteStatus::NotSent(
                                "Docker disconnected; check output before retrying".into(),
                            );
                        }
                        cx.notify();
                    });
                }
                return false;
            }
            _ => return false,
        };
        let Some(s) = self
            .favorites
            .quick
            .sessions
            .iter_mut()
            .find(|s| s.generation == generation)
        else {
            return generation & (1_u64 << 63) != 0;
        };
        s.view.update(cx, |v, cx| {
            match event {
                Event::TerminalReady { .. } => {
                    v.ready = true;
                    v.status = "Connected · interactive quick-run shell".into();
                }
                Event::TerminalData { data, .. } => v.feed(data, cx),
                Event::TerminalPrompt {
                    input_count,
                    at_prompt,
                    ..
                } if *input_count == v.input_count => v.at_prompt = *at_prompt,
                Event::FavoriteStarted { run, .. } => {
                    if let Some(f) = &mut v.favorite
                        && f.id == *run
                    {
                        f.status = FavoriteStatus::Running;
                    }
                }
                Event::FavoriteFinished { run, result, .. } => {
                    if let Some(f) = &mut v.favorite
                        && f.id == *run
                    {
                        f.status = match result {
                            Ok(code) => FavoriteStatus::Completed(*code),
                            Err(error) => FavoriteStatus::NotSent(error.clone()),
                        };
                    }
                }
                Event::TerminalEnded { error, .. } => {
                    v.ready = false;
                    v.at_prompt = false;
                    v.status = error.clone().unwrap_or_else(|| "Shell exited".into());
                    if s.pending.take().is_some() {
                        v.status = format!("{} · command was not sent", v.status);
                    }
                }
                _ => {}
            }
            cx.notify();
        });
        true
    }
    fn close_quick_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.favorites.quick.launcher = None;
        self.favorites.quick.console_open = false;
        self.favorites.quick.error = None;
        if let Some(focus) = self.favorites.quick.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    fn quick_status(s: &Session, cx: &App) -> (String, bool) {
        if s.pending.is_some() {
            return ("Connecting · waiting for shell".into(), false);
        }
        let v = s.view.read(cx);
        match v.favorite.as_ref().map(|f| (&f.label, &f.status)) {
            Some((label, FavoriteStatus::Starting)) => (format!("Sending · {label}"), false),
            Some((label, FavoriteStatus::Running)) => (format!("Running · {label}"), false),
            Some((label, FavoriteStatus::Completed(0))) => {
                (format!("Completed · {label} · exit 0"), false)
            }
            Some((label, FavoriteStatus::Completed(code))) => {
                (format!("Failed · {label} · exit {code}"), true)
            }
            Some((_, FavoriteStatus::NotSent(error))) => (format!("Not sent · {error}"), true),
            None => (v.status.clone(), !v.ready),
        }
    }
}
