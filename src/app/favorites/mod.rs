//! Favorite command controls, editor, target selection and manual terminal launch.
use super::{FerryDock, Tab};
use crate::{
    docker::Command,
    favorites::{Favorite, Mutation, Store, Target},
    terminal::TerminalView,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    IndexPath, WindowExt as _,
    input::{InputState, TextareaState},
    select::{SearchableVec, SelectEvent, SelectState},
};
use std::sync::mpsc;
mod view;

type Picker = Entity<SelectState<SearchableVec<SharedString>>>;
pub(super) struct Editor {
    id: u64,
    label: Entity<InputState>,
    command: Entity<TextareaState>,
}
pub(super) struct Review {
    favorite: Favorite,
    instances: Vec<(String, SharedString)>,
    picker: Picker,
}
pub(crate) struct Favorites {
    items: std::sync::Arc<Vec<Favorite>>,
    requests: mpsc::SyncSender<Option<Mutation>>,
    results: mpsc::Receiver<Result<Store, String>>,
    waiting: bool,
    loaded: bool,
    error: Option<String>,
    scope: Option<(String, Target)>,
    picker: Option<Picker>,
    editor: Option<Editor>,
    review: Option<Review>,
    return_focus: Option<FocusHandle>,
    focus: Option<FocusHandle>,
}
impl Favorites {
    pub(super) fn new() -> Self {
        Self::with_path(Store::path())
    }
    pub(super) fn with_path(path: anyhow::Result<std::path::PathBuf>) -> Self {
        let (requests, receiver) = mpsc::sync_channel(1);
        let (sender, results) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let load = || {
                path.as_ref()
                    .map_err(|error| format!("{error:#}"))
                    .and_then(|path| Store::load(path).map_err(|error| format!("{error:#}")))
            };
            if sender.send(load()).is_err() {
                return;
            }
            while let Ok(mutation) = receiver.recv() {
                let result = if let Some(mutation) = mutation {
                    path.as_ref()
                        .map_err(|error| format!("{error:#}"))
                        .and_then(|path| {
                            Store::mutate(path, mutation).map_err(|error| format!("{error:#}"))
                        })
                } else {
                    load()
                };
                if sender.send(result).is_err() {
                    break;
                }
            }
        });
        Self {
            items: std::sync::Arc::new(vec![]),
            requests,
            results,
            waiting: true,
            loaded: false,
            error: None,
            scope: None,
            picker: None,
            editor: None,
            review: None,
            return_focus: None,
            focus: None,
        }
    }
}

impl FerryDock {
    pub(super) fn favorites_dialog_open(&self) -> bool {
        self.favorites.scope.is_some()
    }
    pub(super) fn poll_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Ok(result) = self.favorites.results.try_recv() else {
            return false;
        };
        self.favorites.waiting = false;
        match result {
            Ok(store) => {
                self.favorites.items = std::sync::Arc::new(store.favorites);
                self.favorites.loaded = true;
                self.favorites.error = None;
                self.favorites.editor = None;
                if self.favorites.scope.is_some()
                    && let Some(focus) = &self.favorites.focus
                {
                    focus.focus(window, cx);
                }
            }
            Err(error) => self.favorites.error = Some(error),
        }
        true
    }

    pub(super) fn container_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(container) = self.selected_container() else {
            return;
        };
        let target = match (&container.project, &container.service) {
            (Some(project), Some(service)) => Target::Service {
                project: project.clone(),
                service: service.clone(),
            },
            (None, _) => Target::Container {
                name: container.name.clone(),
            },
            _ => return,
        };
        self.open_favorites(target, window, cx);
    }
    pub(super) fn project_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.selected.clone() else {
            return;
        };
        let service = self
            .containers
            .items
            .iter()
            .filter(|c| c.project.as_ref() == Some(&project))
            .filter_map(|c| c.service.as_ref())
            .min()
            .cloned()
            .or_else(|| {
                self.favorites
                    .items
                    .iter()
                    .filter(|f| {
                        self.connection
                            .as_ref()
                            .is_some_and(|c| f.endpoint == c.endpoint)
                    })
                    .find_map(|f| match &f.target {
                        Target::Service {
                            project: name,
                            service,
                        } if name == &project => Some(service.clone()),
                        _ => None,
                    })
            })
            .unwrap_or_default();
        self.open_favorites(Target::Service { project, service }, window, cx);
    }
    fn open_favorites(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = &self.connection else {
            return;
        };
        let endpoint = connection.endpoint.clone();
        self.favorites.return_focus = window.focused(cx);
        self.favorites.focus = Some(cx.focus_handle());
        self.favorites.editor = None;
        self.favorites.review = None;
        self.favorites.scope = Some((endpoint.clone(), target.clone()));
        self.favorites.picker = match &target {
            Target::Service { project, service } => {
                let services = self
                    .containers
                    .items
                    .iter()
                    .filter(|c| c.project.as_ref() == Some(project))
                    .filter_map(|c| c.service.clone())
                    .chain(
                        self.favorites
                            .items
                            .iter()
                            .filter(|f| f.endpoint == endpoint)
                            .filter_map(|f| match &f.target {
                                Target::Service {
                                    project: p,
                                    service,
                                } if p == project => Some(service.clone()),
                                _ => None,
                            }),
                    )
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>();
                let index = services
                    .iter()
                    .position(|s| s.as_ref() == service)
                    .map(IndexPath::new);
                let picker = cx.new(|cx| {
                    SelectState::new(SearchableVec::new(services), index, window, cx)
                        .searchable(true)
                });
                cx.subscribe(
                    &picker,
                    |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                        let SelectEvent::Confirm(value) = event;
                        if let Some((_, Target::Service { service, .. })) =
                            &mut this.favorites.scope
                        {
                            *service = value.as_ref().map(ToString::to_string).unwrap_or_default();
                        }
                        this.favorites.editor = None;
                        this.favorites.review = None;
                        cx.notify();
                    },
                )
                .detach();
                Some(picker)
            }
            _ => None,
        };
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            owner
                .update(cx, |this, cx| this.favorites_dialog(dialog, cx))
                .unwrap_or_else(|_| gpui_kit::component::dialog::Dialog::new(cx))
        });
        cx.notify();
    }
    fn close_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.favorites.scope = None;
        self.favorites.picker = None;
        self.favorites.editor = None;
        self.favorites.review = None;
        if let Some(focus) = self.favorites.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }
    fn edit_favorite(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let favorite = id.and_then(|id| self.favorites.items.iter().find(|f| f.id == id));
        let (id, label, command) = favorite
            .map(|f| (f.id, f.label.clone(), f.command.clone()))
            .unwrap_or_default();
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(label)
                .placeholder("e.g. Run tests")
        });
        let command = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .default_value(command)
        });
        label.focus_handle(cx).focus(window, cx);
        self.favorites.editor = Some(Editor { id, label, command });
        self.favorites.review = None;
        cx.notify();
    }
    fn mutate_favorites(&mut self, mutation: Option<Mutation>, cx: &mut Context<Self>) {
        if self.favorites.waiting {
            return;
        }
        match self.favorites.requests.try_send(mutation) {
            Ok(()) => {
                self.favorites.waiting = true;
                self.favorites.error = None;
            }
            Err(error) => self.favorites.error = Some(format!("Cannot update favorites: {error}")),
        }
        cx.notify();
    }
    fn save_favorite(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some((endpoint, target))) =
            (&self.favorites.editor, &self.favorites.scope)
        else {
            return;
        };
        let favorite = Favorite {
            id: editor.id,
            endpoint: endpoint.clone(),
            target: target.clone(),
            label: editor.label.read(cx).value().trim().to_owned(),
            command: editor.command.read(cx).value().to_string(),
        };
        if let Err(error) = favorite.validate() {
            self.favorites.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.mutate_favorites(Some(Mutation::Save(favorite)), cx);
    }
    fn review_favorite(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(favorite) = self.favorites.items.iter().find(|f| f.id == id).cloned() else {
            return;
        };
        let mut instances = self
            .containers
            .items
            .iter()
            .filter(|c| favorite.target.matches(c) && c.running())
            .map(|c| (c.id.clone(), SharedString::from(c.name.clone())))
            .collect::<Vec<_>>();
        instances.sort_by(|a, b| a.1.cmp(&b.1));
        // Replicas require an explicit choice; never pick the first implicitly.
        let index = (instances.len() == 1).then(IndexPath::default);
        let picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    instances
                        .iter()
                        .map(|(_, name)| name.clone())
                        .collect::<Vec<_>>(),
                ),
                index,
                window,
                cx,
            )
        });
        cx.subscribe(
            &picker,
            |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
        )
        .detach();
        self.favorites.review = Some(Review {
            favorite,
            instances,
            picker,
        });
        if let Some(focus) = &self.favorites.focus {
            focus.focus(window, cx);
        }
        self.favorites.editor = None;
        cx.notify();
    }
    fn favorite_run_target(&self, cx: &App) -> Result<String, String> {
        let review = self
            .favorites
            .review
            .as_ref()
            .ok_or("Choose a favorite command")?;
        if self.connection.as_ref().map(|c| &c.endpoint) != Some(&review.favorite.endpoint) {
            return Err("Docker endpoint is disconnected or has changed".into());
        }
        if self.terminal.view.is_some() {
            return Err("A terminal session is already open. Return to Terminal and Disconnect before running a favorite.".into());
        }
        if !self
            .containers
            .items
            .iter()
            .any(|c| review.favorite.target.matches(c) && c.running())
        {
            return Err(if self
                .containers
                .items
                .iter()
                .any(|c| review.favorite.target.matches(c))
            {
                "Service is stopped. Start it, then reopen this command."
            } else {
                "Service or container is absent. Create it, then reopen this command."
            }
            .into());
        }
        if review.instances.is_empty() {
            return Err(
                "New running instances are available. Reopen this command to refresh the choices."
                    .into(),
            );
        }
        let choice = review
            .picker
            .read(cx)
            .selected_value()
            .ok_or("Choose the container instance to run in")?;
        let id = &review
            .instances
            .iter()
            .find(|(_, name)| name == choice)
            .ok_or("Choose a current instance")?
            .0;
        let container = self
            .containers
            .items
            .iter()
            .find(|c| &c.id == id && c.running() && review.favorite.target.matches(c))
            .ok_or("Instance stopped or disappeared. Reopen this command to refresh instances.")?;
        if self.containers.busy.contains_key(id)
            || self.images.pruning.is_some()
            || self.volumes.mutating()
            || container
                .project
                .as_ref()
                .is_some_and(|p| self.project_busy(p))
        {
            return Err(
                "A Docker operation is in progress. Wait before running this command.".into(),
            );
        }
        Ok(id.clone())
    }
    fn run_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(id) = self.favorite_run_target(cx) else {
            return;
        };
        let favorite = self.favorites.review.as_ref().unwrap().favorite.clone();
        window.close_dialog(cx);
        self.close_favorites(window, cx);
        self.select(id.clone(), window, cx);
        self.terminal.generation += 1;
        let generation = self.terminal.generation;
        let commands = self.backend.commands.clone();
        let view = cx.new(|cx| TerminalView::new(generation, commands, cx));
        view.update(cx, |view, cx| view.focus(window, cx));
        self.terminal.view = Some(view);
        self.terminal.command = Some(favorite.command.clone());
        self.backend.send(Command::FavoriteTerminal {
            id,
            generation,
            endpoint: favorite.endpoint,
            target: favorite.target,
            command: favorite.command,
            columns: 80,
            rows: 24,
        });
        self.tab = Tab::Terminal;
        self.expand_dock(580.0, window, cx);
        cx.notify();
    }
}
