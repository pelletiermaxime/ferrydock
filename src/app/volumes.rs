//! Volumes workspace: inventory, measured sizes, selection and guarded removal/prune.
use super::{
    FerryDock,
    ui::{empty_state, palette},
};
use crate::{
    docker::{Command, Event},
    volumes::{PruneMode, Volume, VolumeSize},
};
use gpui::{prelude::*, *};
use gpui_kit::{
    base::TestSupportExt,
    component::{
        IconName, Sizable, WindowExt,
        alert::Alert,
        button::{Button, ButtonVariants},
        dialog::Dialog,
        input::{Input, InputEvent, InputState},
        list::ListItem,
        resizable::{ResizableState, resizable_panel, v_resizable},
        scroll::{Scrollbar, ScrollbarMode},
        spinner::Spinner,
        tag::Tag,
        toolbar::Toolbar,
    },
};
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum VolumeSort {
    #[default]
    Name,
    Driver,
    Age,
    Size,
    References,
}

impl VolumeSort {
    fn id(self) -> &'static str {
        match self {
            Self::Name => "volume-sort-name",
            Self::Driver => "volume-sort-driver",
            Self::Age => "volume-sort-age",
            Self::Size => "volume-sort-size",
            Self::References => "volume-sort-references",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Name => "Volume",
            Self::Driver => "Driver",
            Self::Age => "Age",
            Self::Size => "Size",
            Self::References => "References",
        }
    }
}

pub(super) struct Volumes {
    pub(super) items: Vec<Volume>,
    pub(super) visible: Vec<usize>,
    pub(super) selected: Option<String>,
    pub(super) selection: BTreeSet<String>,
    initialized: bool,
    pub(super) sort: VolumeSort,
    pub(super) descending: bool,
    filter: String,
    search: Entity<InputState>,
    search_focus: FocusHandle,
    pub(super) generation: u64,
    pub(super) loaded: bool,
    loading: bool,
    error: Option<String>,
    sizes: HashMap<String, VolumeSize>,
    sizes_endpoint: Option<String>,
    sizes_measured: Option<chrono::DateTime<chrono::Utc>>,
    sizes_error: Option<String>,
    pub(super) gathering: Option<u64>,
    sizes_request: u64,
    pub(super) confirmation: Option<Vec<String>>,
    pub(super) busy: Option<(Vec<String>, u64)>,
    pub(super) pruning: Option<u64>,
    pub(super) prune_review: bool,
    operation: u64,
    pub(super) scroll: UniformListScrollHandle,
    dock: Entity<ResizableState>,
    pub(super) rows_rendered: Cell<usize>,
}

impl Volumes {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FerryDock>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search name, driver or project…  Ctrl+F")
        });
        let search_focus = search.focus_handle(cx);
        cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.volumes.filter = input.read(cx).value().to_lowercase();
                this.volumes.filter_items();
                this.volumes.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
        })
        .detach();
        let dock = cx.new(|_| ResizableState::default());
        cx.observe(&dock, |_, _, cx| cx.notify()).detach();
        Self {
            items: Vec::new(),
            visible: Vec::new(),
            selected: None,
            selection: BTreeSet::new(),
            initialized: false,
            sort: VolumeSort::default(),
            descending: false,
            filter: String::new(),
            search,
            search_focus,
            generation: 0,
            loaded: false,
            loading: false,
            error: None,
            sizes: HashMap::new(),
            sizes_endpoint: None,
            sizes_measured: None,
            sizes_error: None,
            gathering: None,
            sizes_request: 0,
            confirmation: None,
            busy: None,
            pruning: None,
            prune_review: false,
            operation: 0,
            scroll: UniformListScrollHandle::default(),
            dock,
            rows_rendered: Cell::new(0),
        }
    }

    fn filter_items(&mut self) {
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, volume)| volume.matches(&self.filter).then_some(index))
            .collect();
        let items = &self.items;
        let descending = self.descending;
        let name_key = |index: &usize| {
            (
                items[*index].name.to_lowercase(),
                items[*index].name.clone(),
            )
        };
        match self.sort {
            VolumeSort::Name if descending => self
                .visible
                .sort_by_cached_key(|index| std::cmp::Reverse(name_key(index))),
            VolumeSort::Name => self.visible.sort_by_cached_key(name_key),
            VolumeSort::Driver if descending => self.visible.sort_by_cached_key(|index| {
                (
                    std::cmp::Reverse(items[*index].driver.to_lowercase()),
                    name_key(index),
                )
            }),
            VolumeSort::Driver => self
                .visible
                .sort_by_cached_key(|index| (items[*index].driver.to_lowercase(), name_key(index))),
            VolumeSort::Age => self.visible.sort_by_cached_key(|index| {
                let created = items[*index].created_timestamp();
                let timestamp = i128::from(created.unwrap_or_default());
                (
                    created.is_none(),
                    if descending { timestamp } else { -timestamp },
                    name_key(index),
                )
            }),
            VolumeSort::References => self.visible.sort_by_cached_key(|index| {
                let count = items[*index].containers.len();
                (
                    if descending {
                        usize::MAX - count
                    } else {
                        count
                    },
                    name_key(index),
                )
            }),
            VolumeSort::Size => self.visible.sort_by_cached_key(|index| {
                let size = items[*index].size;
                (
                    size.is_none(),
                    if descending {
                        u64::MAX - size.unwrap_or_default()
                    } else {
                        size.unwrap_or_default()
                    },
                    name_key(index),
                )
            }),
        }
    }

    fn apply_sizes(&mut self, reconcile: bool) {
        if reconcile {
            let items = self
                .items
                .iter()
                .map(|volume| (volume.name.as_str(), volume))
                .collect::<HashMap<_, _>>();
            self.sizes.retain(|name, measurement| {
                items
                    .get(name.as_str())
                    .is_some_and(|volume| measurement.matches(volume))
            });
        }
        for volume in &mut self.items {
            volume.size = self
                .sizes
                .get(&volume.name)
                .filter(|measurement| measurement.matches(volume))
                .and_then(|measurement| measurement.size);
        }
        self.filter_items();
    }

    fn selected_names(&self) -> Vec<String> {
        self.selection.iter().cloned().collect()
    }

    pub(super) fn mutating(&self) -> bool {
        self.busy.is_some() || self.pruning.is_some()
    }

    fn reconcile_selection(&mut self) {
        let existing = self
            .items
            .iter()
            .map(|volume| volume.name.as_str())
            .collect::<HashSet<_>>();
        self.selection
            .retain(|name| existing.contains(name.as_str()));
        if !self.initialized {
            if let Some(index) = self.visible.first() {
                self.selection.insert(self.items[*index].name.clone());
            }
            self.initialized = true;
        }
        if self
            .selected
            .as_ref()
            .is_none_or(|name| !self.selection.contains(name))
        {
            self.selected = self.selection.first().cloned();
        }
    }

    fn selected(&self) -> Option<&Volume> {
        self.selected
            .as_ref()
            .and_then(|name| self.items.iter().find(|volume| &volume.name == name))
    }
}

impl FerryDock {
    fn sort_volumes(&mut self, column: VolumeSort, window: &mut Window, cx: &mut Context<Self>) {
        self.volumes.descending = self.volumes.sort == column && !self.volumes.descending;
        self.volumes.sort = column;
        self.volumes.filter_items();
        self.volumes.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn volume_sort_header(&self, column: VolumeSort, cx: &mut Context<Self>) -> Button {
        let active = self.volumes.sort == column;
        let direction = if self.volumes.descending {
            "descending"
        } else {
            "ascending"
        };
        let colors = palette(cx);
        Button::new(column.id())
            .small()
            .ghost()
            .label(column.label())
            .accessibility_label(if active {
                format!("{}; sorted {direction}", column.label())
            } else {
                format!("Sort by {}", column.label())
            })
            .icon(if !active {
                IconName::ChevronsUpDown
            } else if self.volumes.descending {
                IconName::ArrowDown
            } else {
                IconName::ArrowUp
            })
            .text_color(rgb(if active { colors.accent } else { colors.muted }))
            .tooltip(if active {
                format!("Sorted {direction}; click to reverse")
            } else {
                format!("Sort by {}", column.label())
            })
            .on_click(cx.listener(move |this, _, window, cx| this.sort_volumes(column, window, cx)))
    }

    fn select_volume(
        &mut self,
        name: String,
        toggle: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_volume_review(window, cx);
        if toggle {
            if self.volumes.selection.remove(&name) {
                if self.volumes.selected.as_ref() == Some(&name) {
                    self.volumes.selected = self.volumes.selection.first().cloned();
                }
            } else {
                self.volumes.selection.insert(name.clone());
                self.volumes.selected = Some(name);
            }
        } else {
            self.volumes.selection.clear();
            self.volumes.selection.insert(name.clone());
            self.volumes.selected = Some(name);
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn show_volumes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_networks();
        if self.images_open {
            self.show_images(false, window, cx);
        }
        self.settings_open = false;
        if !self.volumes_open {
            self.volumes_open = true;
            self.refresh_volumes();
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_volumes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.volumes_open {
            return;
        }
        self.volumes_open = false;
        self.volumes.generation += 1;
        self.volumes.loading = false;
        self.cancel_volume_review(window, cx);
        self.backend.send(Command::CloseVolumes);
    }

    pub(super) fn refresh_volumes(&mut self) {
        self.volumes.generation += 1;
        self.volumes.loading = true;
        self.volumes.loaded = false;
        self.backend.send(Command::Volumes {
            generation: self.volumes.generation,
        });
    }

    fn volume_mutation_available(&self) -> bool {
        self.connection.is_some()
            && self.volumes.loaded
            && !self.volumes.loading
            && self.volumes.error.is_none()
            && !self.volumes.mutating()
            && self.images.busy.is_empty()
            && self.images.pruning.is_none()
            && self.containers.busy.is_empty()
            && !self
                .projects
                .operations
                .values()
                .any(|operation| operation.running)
    }

    fn volume_removal_available(&self) -> bool {
        self.volume_mutation_available()
            && self.volumes.items.iter().any(|volume| {
                self.volumes.selection.contains(&volume.name) && volume.containers.is_empty()
            })
    }

    fn cancel_volume_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let removal = self.volumes.confirmation.take().is_some();
        let prune = std::mem::take(&mut self.volumes.prune_review);
        if removal || prune {
            window.close_dialog(cx);
        }
    }

    fn review_volume_prune(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.volume_mutation_available() {
            return;
        }
        self.volumes.prune_review = true;
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let Some(view) = view.upgrade() else {
                return dialog;
            };
            view.update(cx, |this, cx| this.volume_prune_dialog(dialog, cx))
        });
        cx.notify();
    }

    fn prune_volumes(&mut self, mode: PruneMode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.volumes.prune_review || !self.volume_mutation_available() {
            return;
        }
        self.volumes.operation += 1;
        let operation = self.volumes.operation;
        self.volumes.pruning = Some(operation);
        self.volumes.prune_review = false;
        self.volumes.generation += 1;
        self.volumes.loaded = false;
        self.backend.send(Command::PruneVolumes { mode, operation });
        window.close_dialog(cx);
        cx.notify();
    }

    fn volume_prune_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let colors = palette(cx);
        let view = cx.entity().downgrade();
        let daemon = self
            .connection
            .as_ref()
            .map(|connection| connection.name.as_str())
            .unwrap_or("Docker");
        dialog.title("Prune volumes?")
            .width(px(620.0)).overlay_closable(false)
            .on_close(cx.listener(|this, _, _, cx| { this.volumes.prune_review = false; cx.notify(); }))
            .on_cancel(move |_, _, cx| view.update(cx, |this, cx| { this.volumes.prune_review = false; cx.notify(); true }).unwrap_or(true))
            .on_ok(|_, _, _| false)
            .child(div().flex().flex_col().gap_3()
                .child(format!("Remove unused local volumes from {daemon}, regardless of search or selection."))
                .child("Anonymous only removes volumes created without an explicit name. All unused also includes named volumes.")
                .child("All data stored in removed volumes will be permanently deleted. This cannot be undone.")
                .child("Volumes used by running or stopped containers are kept."))
            .footer(div().flex().justify_end().gap_2()
                .child(Self::button("volume-prune-cancel", "Cancel", false, false, cx)
                    .on_click(cx.listener(|this, _, window, cx| { this.cancel_volume_review(window, cx); cx.notify(); })))
                .child(Self::button("volume-prune-anonymous", "Anonymous volumes only", false, !self.volume_mutation_available(), cx)
                    .on_click(cx.listener(|this, _, window, cx| this.prune_volumes(PruneMode::Anonymous, window, cx))))
                .child(Self::button("volume-prune-unused", "All unused volumes", false, !self.volume_mutation_available(), cx)
                    .text_color(rgb(colors.error))
                    .on_click(cx.listener(|this, _, window, cx| this.prune_volumes(PruneMode::AllUnused, window, cx)))))
    }

    fn review_volume_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.volume_removal_available() {
            return;
        }
        self.volumes.confirmation = Some(self.volumes.selected_names());
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            view.update(cx, |this, cx| this.volume_removal_dialog(dialog, cx))
                .unwrap_or_else(|_| Dialog::new(cx).title("Volume unavailable"))
        });
        cx.notify();
    }

    fn remove_reviewed_volume(&mut self, cx: &mut Context<Self>) {
        if !self.volume_removal_available()
            || self.volumes.confirmation.as_ref() != Some(&self.volumes.selected_names())
        {
            return;
        }
        let Some(names) = self.volumes.confirmation.take() else {
            return;
        };
        self.volumes.operation += 1;
        let operation = self.volumes.operation;
        self.volumes.busy = Some((names.clone(), operation));
        // Invalidate snapshots collected before removal began.
        self.volumes.generation += 1;
        self.backend.send(Command::Volumes {
            generation: self.volumes.generation,
        });
        if names.len() == 1 {
            self.backend.send(Command::RemoveVolume {
                name: names[0].clone(),
                operation,
            });
        } else {
            self.backend
                .send(Command::RemoveVolumes { names, operation });
        }
        cx.notify();
    }

    fn volume_removal_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let colors = palette(cx);
        let names = self.volumes.confirmation.clone().unwrap_or_default();
        let volumes = self
            .volumes
            .items
            .iter()
            .filter(|volume| names.binary_search(&volume.name).is_ok())
            .cloned()
            .collect::<Vec<_>>();
        let count = names.len();
        let view = cx.entity().downgrade();
        let confirm = view.clone();
        dialog.title(format!("Remove {count} volume{}?", if count == 1 { "" } else { "s" })).width(px(640.0)).overlay_closable(false)
            .on_close(cx.listener(|this, _, _, cx| { this.volumes.confirmation = None; cx.notify(); }))
            .on_cancel(move |_, _, cx| view.update(cx, |this, cx| { this.volumes.confirmation = None; cx.notify(); true }).unwrap_or(true))
            .on_ok(move |_, _, cx| confirm.update(cx, |this, cx| {
                if !this.volume_removal_available() || this.volumes.confirmation.as_ref() != Some(&this.volumes.selected_names()) { return false; }
                this.remove_reviewed_volume(cx); true
            }).unwrap_or(false))
            .child(div().flex().flex_col().gap_3()
                .child("Permanently delete the selected volumes and all of their stored data? This cannot be undone. Volumes in use will be kept.")
                .child(div().text_sm().text_color(rgb(colors.muted)).child("Running and stopped container references are checked again before removal. Docker removal is never forced."))
                .child(uniform_list("volume-removal-targets", volumes.len(), cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                    range.filter_map(|index| volumes.get(index)).map(|volume| div().id(SharedString::from(format!("volume-removal-target-{}", volume.name))).test_support().h(px(48.0)).flex().items_center().gap_3()
                        .child(div().flex_1().min_w_0().text_ellipsis().child(volume.name.clone()))
                        .child(Tag::secondary().small().outline().child(if volume.containers.is_empty() { "No containers".into() } else { format!("In use · {} container{}", volume.containers.len(), if volume.containers.len() == 1 { "" } else { "s" }) })).into_any_element()).collect::<Vec<_>>()
                })).h(px(48.0 * count.clamp(1, 4) as f32))))
            .footer(div().flex().justify_end().gap_2()
                .child(Self::button("volume-cancel-remove", "Cancel", false, false, cx).on_click(cx.listener(|this, _, window, cx| { this.cancel_volume_review(window, cx); cx.notify(); })))
                .child(Self::button("volume-confirm-remove", format!("Remove {count} volume{}", if count == 1 { "" } else { "s" }), false, !self.volume_removal_available(), cx).text_color(rgb(colors.error))
                    .on_click(cx.listener(|this, _, window, cx| { this.remove_reviewed_volume(cx); if this.volumes.confirmation.is_none() { window.close_dialog(cx); } }))))
    }

    fn finish_volume_removal(
        &mut self,
        operation: u64,
        results: Vec<crate::volumes::RemovalResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((names, active)) = &self.volumes.busy else {
            return;
        };
        if self.connection.is_none() || *active != operation {
            return;
        }
        let results = results
            .into_iter()
            .filter(|result| names.binary_search(&result.name).is_ok())
            .collect::<Vec<_>>();
        let succeeded = results
            .iter()
            .filter(|entry| entry.result.is_ok())
            .map(|entry| entry.name.as_str())
            .collect::<HashSet<_>>();
        self.volumes
            .items
            .retain(|volume| !succeeded.contains(volume.name.as_str()));
        self.volumes
            .selection
            .retain(|name| !succeeded.contains(name.as_str()));
        self.volumes.busy = None;
        self.volumes.filter_items();
        self.volumes.reconcile_selection();
        let removed = succeeded.len();
        let failed = results.len() - removed;
        let errors = results
            .iter()
            .filter_map(|entry| {
                entry
                    .result
                    .as_ref()
                    .err()
                    .map(|error| format!("{}: {error}", entry.name))
            })
            .take(3)
            .collect::<Vec<_>>();
        let mut message = format!(
            "{removed} volume{} removed",
            if removed == 1 { "" } else { "s" }
        );
        if failed > 0 {
            message.push_str(&format!(" · {failed} failed\n{}", errors.join("\n")));
        }
        Self::notify(message, failed > 0, window, cx);
        self.volumes.generation += 1;
        self.volumes.loaded = false;
        self.refresh();
    }

    pub(super) fn volumes_disconnected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.volumes.generation += 1;
        self.volumes.loaded = false;
        self.volumes.loading = false;
        if self.volumes.gathering.take().is_some() {
            self.volumes.sizes_error = Some("Docker disconnected while gathering sizes. Previous measurements are kept; click Gather volume sizes to try again after reconnecting.".into());
        }
        self.cancel_volume_review(window, cx);
        if self.volumes.pruning.take().is_some() {
            Self::notify(
                "Docker disconnected during volume pruning; refresh after reconnecting to check which volumes remain.",
                true,
                window,
                cx,
            );
        }
        if self.volumes.busy.take().is_some() {
            self.volumes.error = Some("Docker disconnected during volume removal; refresh after reconnecting to check which volumes remain.".into());
        }
    }

    pub(super) fn volumes_connected(&mut self, endpoint: &str) {
        if self.volumes.sizes_endpoint.as_deref() != Some(endpoint) {
            self.volumes.sizes.clear();
            self.volumes.sizes_measured = None;
            self.volumes.sizes_error = None;
            self.volumes.gathering = None;
            self.volumes.sizes_endpoint = Some(endpoint.to_owned());
            self.volumes.apply_sizes(false);
        }
    }

    fn gather_volume_sizes(&mut self, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.as_ref() else {
            return;
        };
        let endpoint = connection.endpoint.clone();
        if self.volumes.gathering.is_some() {
            return;
        }
        self.volumes_connected(&endpoint);
        self.volumes.sizes_request += 1;
        let request = self.volumes.sizes_request;
        self.volumes.gathering = Some(request);
        self.volumes.sizes_error = None;
        if self
            .backend
            .commands
            .send(Command::GatherVolumeSizes { request })
            .is_err()
        {
            self.volumes.gathering = None;
            self.volumes.sizes_error =
                Some("Docker worker is unavailable; previous measurements are kept.".into());
        }
        cx.notify();
    }

    pub(super) fn volume_event(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            Event::Volumes { generation, result }
                if generation == self.volumes.generation
                    && self.connection.is_some()
                    && self.volumes_open =>
            {
                self.volumes.loading = false;
                self.volumes.loaded = result.is_ok();
                match result {
                    Ok(items) => {
                        self.volumes.error = None;
                        self.volumes.items = items;
                        self.volumes.apply_sizes(true);
                        self.volumes.reconcile_selection();
                        if self.volumes.confirmation.is_some()
                            && (self.volumes.confirmation.as_ref()
                                != Some(&self.volumes.selected_names())
                                || !self.volume_removal_available())
                        {
                            self.cancel_volume_review(window, cx);
                        }
                    }
                    Err(error) => {
                        self.volumes.error = Some(error);
                        self.cancel_volume_review(window, cx);
                    }
                }
            }
            Event::VolumeRemoved {
                name,
                operation,
                result,
            } => self.finish_volume_removal(
                operation,
                vec![crate::volumes::RemovalResult { name, result }],
                window,
                cx,
            ),
            Event::VolumesRemoved { operation, results } => {
                self.finish_volume_removal(operation, results, window, cx)
            }
            Event::VolumesPruned { operation, result }
                if self.connection.is_some() && self.volumes.pruning == Some(operation) =>
            {
                self.volumes.pruning = None;
                let (message, failed) = match result {
                    Ok(result) => {
                        let deleted = result.deleted.into_iter().collect::<HashSet<_>>();
                        self.volumes
                            .items
                            .retain(|volume| !deleted.contains(&volume.name));
                        self.volumes.sizes.retain(|name, _| !deleted.contains(name));
                        self.volumes.filter_items();
                        self.volumes.reconcile_selection();
                        let space = result
                            .reclaimed
                            .map(|bytes| format!("{:.1} MiB reclaimed", bytes as f64 / 1_048_576.0))
                            .unwrap_or_else(|| "reclaimed space unavailable".into());
                        (
                            format!(
                                "Volume pruning completed · {} volume{} removed · {space}",
                                deleted.len(),
                                if deleted.len() == 1 { "" } else { "s" }
                            ),
                            false,
                        )
                    }
                    Err(error) => (format!("Volume pruning failed: {error}"), true),
                };
                Self::notify(message, failed, window, cx);
                self.volumes.generation += 1;
                self.volumes.loaded = false;
                self.refresh();
            }
            Event::VolumeSizes { request, result }
                if self.volumes.gathering == Some(request) && self.connection.is_some() =>
            {
                self.volumes.gathering = None;
                match result {
                    Ok(sizes) => {
                        self.volumes.sizes = sizes
                            .into_iter()
                            .map(|size| (size.name.clone(), size))
                            .collect();
                        self.volumes.sizes_measured = Some(chrono::Utc::now());
                        self.volumes.sizes_error = None;
                        self.volumes.apply_sizes(false);
                    }
                    Err(error) => {
                        self.volumes.sizes_error =
                            Some(format!("{error} · Previous measurements are kept."))
                    }
                }
            }
            _ => {}
        }
        if self.volumes.prune_review && !self.volume_mutation_available() {
            self.cancel_volume_review(window, cx);
        }
    }

    pub(super) fn volumes_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.volumes.confirmation.is_some() || self.volumes.prune_review {
            return;
        }
        let key = &event.keystroke;
        match key.key.as_str() {
            "f" if key.modifiers.control => self.volumes.search_focus.focus(window, cx),
            "r" if key.modifiers.control => self.refresh(),
            "up" | "down" if self.focus.is_focused(window) => {
                let current = self.volumes.visible.iter().position(|index| {
                    Some(&self.volumes.items[*index].name) == self.volumes.selected.as_ref()
                });
                let next = match current {
                    Some(index) if key.key == "up" => index.saturating_sub(1),
                    Some(index) => (index + 1).min(self.volumes.visible.len().saturating_sub(1)),
                    None => 0,
                };
                if let Some(index) = self.volumes.visible.get(next) {
                    self.select_volume(self.volumes.items[*index].name.clone(), false, window, cx);
                    self.volumes
                        .scroll
                        .scroll_to_item(next, ScrollStrategy::Nearest);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn volume_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        if self.volumes.visible.is_empty() {
            let (title, description) = if self.connection.is_none() {
                (
                    "Waiting for Docker…",
                    "Volumes will reload when Docker reconnects.",
                )
            } else if self.volumes.loading {
                (
                    "Loading volumes…",
                    "Reading volumes and their container references.",
                )
            } else if self.volumes.error.is_some() {
                (
                    "Volumes unavailable",
                    "Refresh to try loading the Docker volume inventory again.",
                )
            } else if !self.volumes.filter.is_empty() {
                (
                    "No volumes match your search",
                    "Search by name, driver or Compose project.",
                )
            } else {
                (
                    "No volumes on this Docker context",
                    "Volumes appear here after creating them with Docker or Compose.",
                )
            };
            return empty_state(title, description).into_any_element();
        }
        let list = uniform_list(
            "volumes-scroll",
            self.volumes.visible.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                this.volumes.rows_rendered.set(range.len());
                range
                    .filter_map(|row| {
                        this.volumes
                            .visible
                            .get(row)
                            .and_then(|index| this.volumes.items.get(*index))
                    })
                    .map(|volume| {
                        let name = volume.name.clone();
                        let selected = this.volumes.selection.contains(&name);
                        ListItem::new(SharedString::from(format!("volume-row-{name}")))
                            .selected(selected)
                            .aria_selected(selected)
                            .accessibility_label(name.clone())
                            .h(px(60.0))
                            .w_full()
                            .flex()
                            .items_center()
                            .px_4()
                            .bg(rgb(if selected {
                                colors.selected
                            } else {
                                colors.background
                            }))
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(div().text_ellipsis().child(name.clone()))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(colors.muted))
                                                    .text_ellipsis()
                                                    .child(
                                                        volume
                                                            .project()
                                                            .unwrap_or("No Compose project")
                                                            .to_owned(),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(80.0))
                                            .flex_shrink_0()
                                            .text_ellipsis()
                                            .child(volume.driver.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("volume-age-{name}")))
                                            .test_support()
                                            .w(px(110.0))
                                            .flex_shrink_0()
                                            .child(
                                                volume.age_label(chrono::Utc::now().timestamp()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("volume-size-{name}")))
                                            .test_support()
                                            .w(px(105.0))
                                            .flex_shrink_0()
                                            .child(volume.size_label()),
                                    )
                                    .child(div().w(px(115.0)).flex_shrink_0().child(
                                        Tag::secondary().small().outline().child(format!(
                                            "{} container{}",
                                            volume.containers.len(),
                                            if volume.containers.len() == 1 {
                                                ""
                                            } else {
                                                "s"
                                            }
                                        )),
                                    )),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.select_volume(
                                    name.clone(),
                                    event.modifiers().control,
                                    window,
                                    cx,
                                )
                            }))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.volumes.scroll)
        .size_full();
        div()
            .id("volumes-viewport")
            .test_support()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(list)
            .child(
                Scrollbar::vertical(&self.volumes.scroll)
                    .id("volumes-scrollbar")
                    .mode(ScrollbarMode::Always),
            )
            .into_any_element()
    }

    fn volume_metadata(
        title: &'static str,
        pairs: &std::collections::BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let rows = std::sync::Arc::new(
            pairs
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>(),
        );
        let count = rows.len();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().font_weight(FontWeight::MEDIUM).child(title))
            .when(count == 0, |view| view.child(div().text_sm().child("None")))
            .when(count > 0, |view| {
                view.child(
                    uniform_list(
                        SharedString::from(format!("volume-{title}")),
                        count,
                        cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                            range
                                .filter_map(|index| rows.get(index))
                                .map(|(key, value)| {
                                    div()
                                        .h(px(28.0))
                                        .text_sm()
                                        .text_ellipsis()
                                        .child(format!("{key} = {value}"))
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .h(px(28.0 * count.min(3) as f32))
                    .flex_shrink_0(),
                )
            })
    }

    fn volume_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(volume) = self.volumes.selected() else {
            return empty_state(
                "Select a volume",
                "Choose a volume to see its configuration and container references.",
            )
            .into_any_element();
        };
        let references = volume.containers.clone();
        div().id("volume-detail").test_support().size_full().flex().flex_col().bg(rgb(colors.panel))
            .child(Toolbar::new("volume-detail-toolbar").px_5().py_3().border_b_1().border_color(rgb(colors.border))
                .content(div().flex_1().min_w_0().font_weight(FontWeight::MEDIUM).text_ellipsis().child(volume.name.clone())))
            .child(div().id("volume-detail-scroll").flex_1().min_h_0().overflow_y_scroll().px_5().py_3().flex().flex_col().gap_3()
                .child(div().text_sm().child(format!("Driver: {} · Scope: {} · Size: {}", volume.driver, volume.scope.as_deref().filter(|scope| !scope.is_empty()).unwrap_or("Unavailable"), volume.size_label())))
                .child(div().text_sm().child(format!("Created: {}", volume.created_label())))
                .child(div().text_xs().text_color(rgb(colors.muted)).child("Click Gather volume sizes to measure disk usage. Measurements stay cached until you gather them again; unsupported volumes show Unavailable."))
                .child(div().text_sm().child(format!("Mountpoint (Docker host): {}", if volume.mountpoint.is_empty() { "Unavailable" } else { &volume.mountpoint })))
                .child(div().font_weight(FontWeight::MEDIUM).child(format!("Containers using this volume ({})", references.len())))
                .when(references.is_empty(), |view| view.child(div().text_sm().text_color(rgb(colors.muted)).child("No containers reference this volume. References are checked again before removal.")))
                .when(!references.is_empty(), |view| view.child(uniform_list("volume-containers", references.len(), cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range.filter_map(|index| references.get(index)).map(|container| {
                        let id = container.id.clone();
                        ListItem::new(SharedString::from(format!("volume-container-{id}"))).h(px(36.0)).px_2()
                            .child(div().w_full().flex().items_center().gap_3().child(div().flex_1().min_w_0().text_ellipsis().child(container.name.clone())).child(Tag::secondary().small().outline().child(container.state.clone())))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(container) = this.containers.items.iter().find(|container| container.id == id) else { this.refresh(); return; };
                                let stopped = !container.running();
                                this.show_images(false, window, cx);
                                if stopped { this.containers.running_only = false; }
                                this.containers.search_input.update(cx, |input, cx| input.set_value("", window, cx));
                                this.select(id.clone(), window, cx);
                                this.focus.focus(window, cx);
                            })).into_any_element()
                    }).collect::<Vec<_>>()
                })).h(px(108.0)).flex_shrink_0()))
                .when(!volume.containers.is_empty(), |view| view.child(div().text_sm().text_color(rgb(colors.muted)).child("Removal is unavailable while any running or stopped container uses this volume.")))
                .child(Self::volume_metadata("Labels", &volume.labels, cx))
                .child(Self::volume_metadata("Driver options", &volume.options, cx)))
            .into_any_element()
    }

    pub(super) fn volumes_workspace(&self, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let busy = self.volumes.busy.is_some();
        let count = self.volumes.selection.len();
        let hidden = count
            - self
                .volumes
                .visible
                .iter()
                .filter(|index| {
                    self.volumes
                        .selection
                        .contains(&self.volumes.items[**index].name)
                })
                .count();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                Toolbar::new("volumes-toolbar")
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .content(div().font_weight(FontWeight::MEDIUM).child("Volumes"))
                    .content(Tag::secondary().small().outline().child(format!(
                        "{} / {} volumes",
                        self.volumes.visible.len(),
                        self.volumes.items.len()
                    )))
                    .when(count > 0, |toolbar| toolbar.content(Tag::secondary().small().outline().child(format!("{count} selected{}", if hidden > 0 { format!(" · {hidden} hidden") } else { String::new() }))))
                    .when(self.volumes.loading, |toolbar| {
                        toolbar.content(Spinner::new().small())
                    })
                    .content(div().flex_1())
                    .child(
                        Self::button("volume-gather-sizes", "Gather volume sizes", false, self.connection.is_none() || self.volumes.gathering.is_some(), cx)
                            .loading(self.volumes.gathering.is_some())
                            .tooltip("Calculate volume disk usage. Normal refreshes keep the last measurements.")
                            .on_click(cx.listener(|this, _, _, cx| this.gather_volume_sizes(cx))),
                    )
                    .child(Self::button("volume-prune", if self.volumes.pruning.is_some() { "Pruning…" } else { "Prune…" }, false, !self.volume_mutation_available(), cx)
                        .loading(self.volumes.pruning.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.review_volume_prune(window, cx))))
                    .content(
                        div()
                            .id("volume-search")
                            .test_support()
                            .w(px(320.0))
                            .capture_key_down(cx.listener(Self::search_key))
                            .child(Input::new(&self.volumes.search).small()),
                    ),
            )
            .child(
                Toolbar::new("volume-actions")
                    .px_5()
                    .py_2()
                    .content(div().flex_1().min_w_0().text_xs().text_ellipsis().text_color(rgb(colors.muted)).child(
                        if self.volumes.gathering.is_some() { "Gathering sizes…".to_owned() }
                        else if let Some(measured) = self.volumes.sizes_measured { format!("Measured {}", measured.format("%Y-%m-%d %H:%M UTC")) }
                        else { "Sizes have not been gathered.".to_owned() }
                    ))
                    .child(
                        Self::button(
                            "volume-remove",
                            if let Some((names, _)) = &self.volumes.busy { format!("Removing {} volumes…", names.len()) }
                            else if count > 1 { format!("Remove selected ({count})…") }
                            else { "Remove volume…".into() },
                            false,
                            !self.volume_removal_available(),
                            cx,
                        )
                        .loading(busy)
                        .text_color(rgb(colors.error))
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.review_volume_removal(window, cx)
                            }),
                        ),
                    ),
            )
            .child(div().px_5().pb_2().text_xs().text_color(rgb(colors.muted)).child("Ctrl-click to add or remove volumes from the selection. A normal click selects one."))
            .when_some(self.volumes.sizes_error.clone(), |view, error| {
                view.child(Alert::error("volume-sizes-error", error))
            })
            .when_some(self.connection_error.clone(), |view, error| {
                view.child(Alert::error(
                    "volumes-connection-error",
                    format!("{error} · reconnecting…"),
                ))
            })
            .when_some(self.volumes.error.clone(), |view, error| {
                view.child(Alert::error("volumes-error", error))
            })
            .child(
                div().flex_1().min_h_0().child(
                    v_resizable("volumes-split")
                        .with_state(&self.volumes.dock)
                        .child(
                            resizable_panel().size_range(px(80.0)..Pixels::MAX).child(
                                div()
                                    .size_full()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .px_4()
                                            .py_1()
                                            .text_xs()
                                            .text_color(rgb(colors.muted))
                                            .child(div().flex_1().flex().child(self.volume_sort_header(VolumeSort::Name, cx)))
                                            .child(div().w(px(80.0)).child(self.volume_sort_header(VolumeSort::Driver, cx)))
                                            .child(div().w(px(110.0)).child(self.volume_sort_header(VolumeSort::Age, cx)))
                                            .child(div().w(px(105.0)).child(self.volume_sort_header(VolumeSort::Size, cx)))
                                            .child(div().w(px(115.0)).child(self.volume_sort_header(VolumeSort::References, cx))),
                                    )
                                    .child(
                                        div()
                                            .id("volumes-pane")
                                            .test_support()
                                            .flex_1()
                                            .min_h_0()
                                            .child(self.volume_list(cx)),
                                    ),
                            ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(360.0))
                                .size_range(px(280.0)..Pixels::MAX)
                                .child(self.volume_detail(cx)),
                        ),
                ),
            )
    }
}
