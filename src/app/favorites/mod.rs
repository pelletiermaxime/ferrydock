//! Favorite command controls, editor, target selection and manual terminal launch.
use super::FerryDock;
use crate::favorites::{Favorite, Mutation, Store, Target};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    IndexPath, WindowExt as _,
    input::{InputState, TextareaState},
    select::{SearchableVec, SelectEvent, SelectState},
};
use std::sync::mpsc;
mod quick_run;
mod terminal_controls;
mod view;

type Picker = Entity<SelectState<SearchableVec<SharedString>>>;
pub(super) struct Editor {
    id: u64,
    label: Entity<InputState>,
    command: Entity<TextareaState>,
}
pub(crate) struct Favorites {
    items: std::sync::Arc<Vec<Favorite>>,
    quick: quick_run::QuickRun,
    used_pending: std::collections::BTreeSet<u64>,
    metadata_save: bool,
    requests: mpsc::SyncSender<Option<Mutation>>,
    results: mpsc::Receiver<Result<Store, String>>,
    waiting: bool,
    loaded: bool,
    error: Option<String>,
    scope: Option<(String, Target)>,
    picker: Option<Picker>,
    editor: Option<Editor>,
    terminal_picker: Option<Entity<SelectState<SearchableVec<terminal_controls::Choice>>>>,
    terminal_key: Option<(String, Target, usize)>,
    next_run: u64,
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
            quick: quick_run::QuickRun::default(),
            used_pending: Default::default(),
            metadata_save: false,
            requests,
            results,
            waiting: true,
            loaded: false,
            error: None,
            scope: None,
            picker: None,
            editor: None,
            terminal_picker: None,
            terminal_key: None,
            next_run: 0,
            return_focus: None,
            focus: None,
        }
    }
}

impl FerryDock {
    #[cfg(target_os = "linux")]
    pub(super) fn favorite_save_pending(&self) -> bool {
        self.favorites.waiting || !self.favorites.used_pending.is_empty()
    }
    fn favorite_operation_conflict(&self, container: &crate::model::Container) -> bool {
        // Match the worker's RunFavorite guard: a sibling container operation
        // does not affect this shell, but a Compose operation on its project can.
        self.containers.busy.contains_key(&container.id)
            || self.volumes.mutating()
            || self.images.pruning.is_some()
            || container.project.as_ref().is_some_and(|project| {
                self.projects
                    .operations
                    .get(project)
                    .is_some_and(|operation| operation.running)
            })
    }

    pub(super) fn favorites_dialog_open(&self) -> bool {
        self.favorites.scope.is_some()
    }
    pub(super) fn poll_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let changed = self.sync_terminal_favorites(window, cx) | self.poll_quick_runs(cx);
        if !self.favorites.waiting
            && let Some(id) = self.favorites.used_pending.pop_first()
        {
            self.mutate_favorites(Some(Mutation::Used(id)), cx);
            self.favorites.metadata_save = true;
        }
        let Ok(result) = self.favorites.results.try_recv() else {
            return changed;
        };
        self.favorites.waiting = false;
        match result {
            Ok(store) => {
                self.favorites.items = std::sync::Arc::new(store.favorites);
                self.favorites.loaded = true;
                self.favorites.error = None;
                if !self.favorites.metadata_save {
                    self.favorites.editor = None;
                }
                if !self.favorites.metadata_save
                    && self.favorites.scope.is_some()
                    && let Some(focus) = &self.favorites.focus
                {
                    focus.focus(window, cx);
                }
            }
            Err(error) => self.favorites.error = Some(error),
        }
        self.favorites.metadata_save = false;
        self.sync_terminal_favorites(window, cx);
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
        cx.notify();
    }
    fn mutate_favorites(&mut self, mutation: Option<Mutation>, cx: &mut Context<Self>) {
        if self.favorites.waiting {
            return;
        }
        match self.favorites.requests.try_send(mutation) {
            Ok(()) => {
                self.favorites.waiting = true;
                self.favorites.metadata_save = false;
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
            pinned: self
                .favorites
                .items
                .iter()
                .find(|f| f.id == editor.id)
                .is_some_and(|f| f.pinned),
            last_used: self
                .favorites
                .items
                .iter()
                .find(|f| f.id == editor.id)
                .map_or(0, |f| f.last_used),
            use_count: self
                .favorites
                .items
                .iter()
                .find(|f| f.id == editor.id)
                .map_or(0, |f| f.use_count),
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
}
