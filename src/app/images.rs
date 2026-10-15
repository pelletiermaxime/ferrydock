//! Images workspace: discovery state, search, selection, references, and removal.
use std::{
    cell::Cell,
    collections::{BTreeSet, HashMap},
};

use super::{
    FerryDock,
    image_details::{ImageDetails, ImageTab},
    ui::{empty_state, palette},
};
use crate::{
    docker::{Command, Event},
    images::{Image, PruneMode},
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ImageSort {
    #[default]
    Name,
    Size,
    Age,
    References,
}

impl ImageSort {
    fn id(self) -> &'static str {
        match self {
            Self::Name => "image-sort-name",
            Self::Size => "image-sort-size",
            Self::Age => "image-sort-age",
            Self::References => "image-sort-references",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Image / ID",
            Self::Size => "Size (MiB)",
            Self::Age => "Age",
            Self::References => "References",
        }
    }
}

pub(super) struct Images {
    pub(super) items: Vec<Image>,
    pub(super) visible: Vec<usize>,
    pub(super) selected: Option<String>,
    pub(super) selection: BTreeSet<String>,
    initialized: bool,
    pub(super) filter: String,
    pub(super) sort: ImageSort,
    pub(super) descending: bool,
    pub(super) search: Entity<InputState>,
    pub(super) search_focus: FocusHandle,
    pub(super) generation: u64,
    pub(super) loading: bool,
    pub(super) loaded: bool,
    pub(super) error: Option<String>,
    pub(super) confirmation: Option<Vec<String>>,
    pub(super) prune_review: bool,
    pub(super) pruning: Option<u64>,
    pub(super) busy: HashMap<String, u64>,
    removing_names: HashMap<String, String>,
    pub(super) operation: u64,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) dock: Entity<ResizableState>,
    pub(super) rows_rendered: Cell<usize>,
    pub(super) details: ImageDetails,
}

impl Images {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FerryDock>) -> Self {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search tags or image ID…  Ctrl+F"));
        let search_focus = search.focus_handle(cx);
        cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.images.filter = input.read(cx).value().to_string();
                this.images.filter_items();
                this.images.scroll.scroll_to_item(0, ScrollStrategy::Top);
                this.images.confirmation = None;
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
            filter: String::new(),
            sort: ImageSort::default(),
            descending: false,
            search,
            search_focus,
            generation: 0,
            loading: false,
            loaded: false,
            error: None,
            confirmation: None,
            prune_review: false,
            pruning: None,
            busy: HashMap::new(),
            removing_names: HashMap::new(),
            operation: 0,
            scroll: UniformListScrollHandle::default(),
            dock,
            rows_rendered: Cell::new(0),
            details: ImageDetails::new(window, cx),
        }
    }

    fn filter_items(&mut self) {
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, image)| image.matches(&self.filter).then_some(index))
            .collect();
        let items = &self.items;
        let descending = self.descending;
        let name_key = |index: &usize| {
            (
                items[*index].name().to_lowercase(),
                items[*index].id.clone(),
            )
        };
        match self.sort {
            ImageSort::Name if descending => self
                .visible
                .sort_by_cached_key(|index| std::cmp::Reverse(name_key(index))),
            ImageSort::Name => self.visible.sort_by_cached_key(name_key),
            ImageSort::Size => self.visible.sort_by_cached_key(|index| {
                let size = items[*index].size;
                let bytes = size.unwrap_or_default();
                (
                    size.is_none(),
                    if descending { u64::MAX - bytes } else { bytes },
                    name_key(index),
                )
            }),
            ImageSort::Age => self.visible.sort_by_cached_key(|index| {
                let created = items[*index].created_timestamp();
                let timestamp = created.unwrap_or_default();
                (
                    created.is_none(),
                    if !descending {
                        i64::MAX - timestamp
                    } else {
                        timestamp
                    },
                    name_key(index),
                )
            }),
            ImageSort::References => self.visible.sort_by_cached_key(|index| {
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
        }
    }

    fn selected(&self) -> Option<&Image> {
        self.selected
            .as_ref()
            .and_then(|id| self.items.iter().find(|image| &image.id == id))
    }

    fn selected_ids(&self) -> Vec<String> {
        self.selection.iter().cloned().collect()
    }

    fn reconcile_selection(&mut self) {
        let existing = self
            .items
            .iter()
            .map(|image| image.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.selection.retain(|id| existing.contains(id.as_str()));
        if !self.initialized {
            if let Some(index) = self.visible.first() {
                self.selection.insert(self.items[*index].id.clone());
            }
            self.initialized = true;
        }
        if self
            .selected
            .as_ref()
            .is_none_or(|id| !self.selection.contains(id))
        {
            self.selected = self.selection.first().cloned();
        }
        if self
            .confirmation
            .as_ref()
            .is_some_and(|ids| *ids != self.selected_ids())
        {
            self.confirmation = None;
        }
    }

    pub(super) fn interrupt_removal(&mut self) {
        if !self.busy.is_empty() || self.pruning.is_some() {
            self.error = Some("Docker disconnected during image cleanup; refresh after reconnecting to check which images remain.".into());
        }
        self.pruning = None;
        self.prune_review = false;
        self.busy.clear();
        self.removing_names.clear();
    }
}

impl FerryDock {
    fn sort_images(&mut self, column: ImageSort, window: &mut Window, cx: &mut Context<Self>) {
        self.images.descending = self.images.sort == column && !self.images.descending;
        self.images.sort = column;
        self.images.filter_items();
        self.images.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn image_sort_header(&self, column: ImageSort, cx: &mut Context<Self>) -> Button {
        let active = self.images.sort == column;
        let colors = palette(cx);
        let direction = if self.images.descending {
            "descending"
        } else {
            "ascending"
        };
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
            } else if self.images.descending {
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
            .on_click(cx.listener(move |this, _, window, cx| this.sort_images(column, window, cx)))
    }

    pub(super) fn show_images(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_volumes(window, cx);
        self.close_networks();
        let changed = self.images_open != open;
        self.images_open = open;
        self.settings_open = false;
        if changed {
            self.images.confirmation = None;
            if open {
                self.refresh_images();
            } else {
                self.invalidate_image_details(window, cx);
                self.images.generation += 1;
                self.images.loading = false;
                self.backend.send(Command::CloseImages);
            }
        }
        if open {
            self.focus.focus(window, cx);
        } else {
            self.show_settings(false, window, cx);
        }
        cx.notify();
    }

    pub(super) fn refresh(&mut self) {
        self.backend.send(Command::Refresh);
        if self.images_open {
            self.refresh_images();
        }
        if self.volumes_open {
            self.refresh_volumes();
        }
        if self.networks_open {
            self.refresh_networks();
        }
    }

    fn refresh_images(&mut self) {
        if self.images.details.requested {
            self.backend.send(Command::CloseImageDetails);
        }
        if self.images.details.analysis.loading {
            self.backend.send(Command::CloseImageAnalysis);
        }
        self.images.details.reset();
        self.images.generation += 1;
        self.images.loading = true;
        self.images.loaded = false;
        self.images.confirmation = None;
        self.backend.send(Command::Images {
            generation: self.images.generation,
        });
    }

    fn select_image(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.images.selection.len() != 1 || !self.images.selection.contains(&id) {
            self.images.confirmation = None;
        }
        self.images.selection.clear();
        self.images.selection.insert(id.clone());
        self.images.selected = Some(id);
        self.sync_image_details(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn toggle_image_selection(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.images.confirmation = None;
        if self.images.selection.remove(&id) {
            if self.images.selected.as_ref() == Some(&id) {
                self.images.selected = self.images.selection.first().cloned();
            }
        } else {
            self.images.selection.insert(id.clone());
            self.images.selected = Some(id);
        }
        self.sync_image_details(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn image_removal_available(&self) -> bool {
        self.connection.is_some()
            && !self.volumes.mutating()
            && self.images.loaded
            && !self.images.loading
            && self.images.error.is_none()
            && self.images.busy.is_empty()
            && self.images.pruning.is_none()
            && !self
                .projects
                .operations
                .values()
                .any(|operation| operation.running)
            && self.images.items.iter().any(|image| {
                self.images.selection.contains(&image.id) && image.containers.is_empty()
            })
    }

    fn image_prune_available(&self) -> bool {
        self.connection.is_some()
            && self.images.loaded
            && !self.images.loading
            && self.images.error.is_none()
            && self.images.busy.is_empty()
            && self.images.pruning.is_none()
            && !self.volumes.mutating()
            && self.containers.busy.is_empty()
            && !self
                .projects
                .operations
                .values()
                .any(|operation| operation.running)
    }

    fn review_image_prune(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.image_prune_available() {
            return;
        }
        self.images.prune_review = true;
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let Some(view) = view.upgrade() else {
                return dialog;
            };
            view.update(cx, |this, cx| this.image_prune_dialog(dialog, cx))
        });
        cx.notify();
    }

    fn prune_images(&mut self, mode: PruneMode, window: &mut Window, cx: &mut Context<Self>) {
        if !self.images.prune_review || !self.image_prune_available() {
            return;
        }
        self.images.operation += 1;
        let operation = self.images.operation;
        self.images.pruning = Some(operation);
        self.images.prune_review = false;
        self.backend.send(Command::PruneImages { mode, operation });
        window.close_dialog(cx);
        cx.notify();
    }

    fn image_prune_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let colors = palette(cx);
        let cancel_view = cx.entity().downgrade();
        dialog.title("Prune images?")
            .width(px(620.0))
            .overlay_closable(false)
            .on_close(cx.listener(|this, _, _, cx| { this.images.prune_review = false; cx.notify(); }))
            .on_cancel(move |_, _, cx| cancel_view.update(cx, |this, cx| { this.images.prune_review = false; cx.notify(); true }).unwrap_or(true))
            // Enter on the dialog itself must never choose a destructive mode.
            .on_ok(|_, _, _| false)
            .child(div().flex().flex_col().gap_3()
                .child("Remove unused images from the connected Docker daemon, regardless of search or selection.")
                .child("Untagged removes only images without tags. All unused also includes tagged images.")
                .child("Images used by running or stopped containers are kept."))
            .footer(div().flex().justify_end().gap_2()
                .child(Self::button("image-prune-cancel", "Cancel", false, false, cx)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.images.prune_review = false;
                        window.close_dialog(cx);
                        cx.notify();
                    })))
                .child(Self::button("image-prune-untagged", "All untagged images", false, !self.image_prune_available(), cx)
                    .on_click(cx.listener(|this, _, window, cx| this.prune_images(PruneMode::Untagged, window, cx))))
                .child(Self::button("image-prune-unused", "All unused images", false, !self.image_prune_available(), cx)
                    .text_color(rgb(colors.error))
                    .on_click(cx.listener(|this, _, window, cx| this.prune_images(PruneMode::AllUnused, window, cx)))))
    }

    fn review_image_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_removal_available() {
            self.images.confirmation = Some(self.images.selected_ids());
            let view = cx.entity().downgrade();
            window.open_dialog(cx, move |dialog, _, cx| {
                let Some(view) = view.upgrade() else {
                    return dialog;
                };
                view.update(cx, |this, cx| this.image_removal_dialog(dialog, cx))
            });
            cx.notify();
        }
    }

    fn remove_selected_image(&mut self, cx: &mut Context<Self>) {
        if !self.image_removal_available()
            || self.images.confirmation.as_ref() != Some(&self.images.selected_ids())
        {
            return;
        }
        let ids = self.images.selected_ids();
        self.images.operation += 1;
        let operation = self.images.operation;
        self.images.removing_names = self
            .images
            .items
            .iter()
            .filter(|image| self.images.selection.contains(&image.id))
            .map(|image| (image.id.clone(), image.name().to_owned()))
            .collect();
        for id in &ids {
            self.images.busy.insert(id.clone(), operation);
        }
        self.images.confirmation = None;
        if ids.len() == 1 {
            self.backend.send(Command::RemoveImage {
                id: ids[0].clone(),
                operation,
            });
        } else {
            self.backend.send(Command::RemoveImages { ids, operation });
        }
        cx.notify();
    }

    fn finish_image_removal(
        &mut self,
        operation: u64,
        results: Vec<crate::images::RemovalResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.connection.is_none()
            || !self.images.busy.values().any(|active| *active == operation)
        {
            return;
        }
        let results = results
            .into_iter()
            .filter(|result| self.images.busy.get(&result.id) == Some(&operation))
            .collect::<Vec<_>>();
        let succeeded = results
            .iter()
            .filter(|entry| entry.result.is_ok())
            .map(|entry| entry.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        self.images
            .items
            .retain(|image| !succeeded.contains(image.id.as_str()));
        for result in &results {
            self.images.busy.remove(&result.id);
            if result.result.is_ok() {
                self.images.selection.remove(&result.id);
            }
        }
        self.images.filter_items();
        self.images.reconcile_selection();
        let removed = succeeded.len();
        let failed = results.len() - removed;
        let errors = results
            .into_iter()
            .filter_map(|entry| {
                let name = self
                    .images
                    .removing_names
                    .remove(&entry.id)
                    .unwrap_or(entry.id);
                entry.result.err().map(|error| format!("{name}: {error}"))
            })
            .take(3)
            .collect::<Vec<_>>();
        self.images.removing_names.clear();
        let mut message = format!(
            "{removed} image{} removed",
            if removed == 1 { "" } else { "s" }
        );
        if failed > 0 {
            message.push_str(&format!(" · {failed} failed\n{}", errors.join("\n")));
        }
        Self::notify(message, failed > 0, window, cx);
        self.refresh();
    }

    pub(super) fn image_event(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_confirming = self.images.confirmation.is_some();
        match event {
            event @ Event::ImageAnalysis { .. } => self.image_analysis_event(event, cx),
            event @ (Event::ImageHistory { .. } | Event::ImageInspection { .. }) => {
                self.image_details_event(event, window, cx);
            }
            Event::Images { generation, result }
                if generation == self.images.generation && self.connection.is_some() =>
            {
                self.images.loading = false;
                self.images.loaded = result.is_ok();
                match result {
                    Ok(items) => {
                        self.images.error = None;
                        self.images.items = items;
                        self.images.filter_items();
                        self.images.reconcile_selection();
                        self.sync_image_details(window, cx);
                        if !self.images.items.iter().any(|image| {
                            self.images.selection.contains(&image.id) && image.containers.is_empty()
                        }) {
                            self.images.confirmation = None;
                        }
                    }
                    Err(error) => {
                        self.images.error = Some(error);
                        self.images.confirmation = None;
                    }
                }
            }
            Event::ImageRemoved {
                id,
                operation,
                result,
            } if self.connection.is_some() && self.images.busy.get(&id) == Some(&operation) => {
                self.finish_image_removal(
                    operation,
                    vec![crate::images::RemovalResult { id, result }],
                    window,
                    cx,
                );
            }
            Event::ImagesRemoved { operation, results } => {
                self.finish_image_removal(operation, results, window, cx)
            }
            Event::ImagesPruned { operation, result }
                if self.connection.is_some() && self.images.pruning == Some(operation) =>
            {
                self.images.pruning = None;
                let (message, failed) = match result {
                    Ok(Some(bytes)) => (
                        format!(
                            "Image pruning completed · {:.1} MiB reclaimed",
                            bytes as f64 / 1_048_576.0
                        ),
                        false,
                    ),
                    Ok(None) => (
                        "Image pruning completed · reclaimed space unavailable".into(),
                        false,
                    ),
                    Err(error) => (format!("Image pruning failed: {error}"), true),
                };
                Self::notify(message, failed, window, cx);
                self.refresh();
            }
            _ => {}
        }
        if was_confirming && self.images.confirmation.is_none() {
            window.close_dialog(cx);
        }
        if self.images.prune_review && !self.image_prune_available() {
            self.images.prune_review = false;
            window.close_dialog(cx);
        }
    }

    fn image_removal_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let colors = palette(cx);
        let ids = self.images.confirmation.clone().unwrap_or_default();
        let images = self
            .images
            .items
            .iter()
            .filter(|image| ids.binary_search(&image.id).is_ok())
            .cloned()
            .collect::<Vec<_>>();
        let count = ids.len();
        let cancel_view = cx.entity().downgrade();
        let confirm_view = cancel_view.clone();
        dialog.title(format!("Remove {count} image{}?", if count == 1 { "" } else { "s" }))
            .width(px(640.0))
            .overlay_closable(false)
            .on_close(cx.listener(|this, _, _, cx| { this.images.confirmation = None; cx.notify(); }))
            .on_cancel(move |_, _, cx| cancel_view.update(cx, |this, cx| { this.images.confirmation = None; cx.notify(); true }).unwrap_or(true))
            .on_ok(move |_, _, cx| confirm_view.update(cx, |this, cx| {
                if !this.image_removal_available() || this.images.confirmation.as_ref() != Some(&this.images.selected_ids()) { return false; }
                this.remove_selected_image(cx);
                true
            }).unwrap_or(false))
            .child(div().flex().flex_col().gap_3()
                .child("Remove the selected images and their local tags? You may need to pull or rebuild them again. Images used by containers will be kept.")
                .child(uniform_list("image-removal-targets", images.len(), cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                    range.filter_map(|index| images.get(index)).map(|image| div().h(px(48.0)).flex().flex_col().justify_center()
                        .child(div().flex().gap_2().child(div().flex_1().min_w_0().text_ellipsis().child(image.name().to_owned()))
                            .child(Tag::secondary().small().outline().child(if image.containers.is_empty() { "No containers".into() } else { format!("In use · {} container{}", image.containers.len(), if image.containers.len() == 1 { "" } else { "s" }) })))
                        .child(div().text_xs().text_color(rgb(colors.muted)).text_ellipsis().child(image.id.clone())).into_any_element()).collect::<Vec<_>>()
                })).h(px(48.0 * count.clamp(1, 4) as f32))))
            .footer(div().flex().justify_end().gap_2()
                .child(Self::button("image-cancel-remove", "Cancel", false, false, cx).on_click(cx.listener(|this, _, window, cx| {
                    this.images.confirmation = None;
                    window.close_dialog(cx);
                    cx.notify();
                })))
                .child(Self::button("image-confirm-remove", format!("Remove {count} image{}", if count == 1 { "" } else { "s" }), false, !self.image_removal_available() || count == 0, cx)
                    .text_color(rgb(colors.error)).on_click(cx.listener(|this, _, window, cx| {
                        this.remove_selected_image(cx);
                        if this.images.confirmation.is_none() { window.close_dialog(cx); }
                    }))))
    }

    pub(super) fn images_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.images.confirmation.is_some() || self.images.prune_review {
            return;
        }
        if self.history_key(event, window, cx) {
            return;
        }
        let key = &event.keystroke;
        match key.key.as_str() {
            "1" if key.modifiers.control => self.switch_image_tab(ImageTab::Overview, window, cx),
            "2" if key.modifiers.control => self.switch_image_tab(ImageTab::History, window, cx),
            "3" if key.modifiers.control => self.switch_image_tab(ImageTab::Inspect, window, cx),
            "4" if key.modifiers.control => self.switch_image_tab(ImageTab::Analysis, window, cx),
            "f" if key.modifiers.control => self.images.search_focus.focus(window, cx),
            "r" if key.modifiers.control => self.refresh(),
            "escape" => {
                self.images.confirmation = None;
                self.focus.focus(window, cx);
            }
            "up" | "down" if self.focus.is_focused(window) => {
                let current = self.images.visible.iter().position(|index| {
                    Some(&self.images.items[*index].id) == self.images.selected.as_ref()
                });
                let next = match current {
                    Some(current) if key.key == "up" => current.saturating_sub(1),
                    Some(current) => (current + 1).min(self.images.visible.len().saturating_sub(1)),
                    None => 0,
                };
                if let Some(index) = self.images.visible.get(next) {
                    self.select_image(self.images.items[*index].id.clone(), window, cx);
                    self.images
                        .scroll
                        .scroll_to_item(next, ScrollStrategy::Nearest);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn image_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        if self.images.visible.is_empty() {
            let (title, description) = if self.connection.is_none() {
                (
                    "Waiting for Docker…",
                    "Images will reload when Docker reconnects.",
                )
            } else if self.images.loading {
                (
                    "Loading images…",
                    "Reading images and their container references.",
                )
            } else if self.images.error.is_some() {
                (
                    "Images unavailable",
                    "Refresh to try loading the Docker image inventory again.",
                )
            } else if !self.images.filter.is_empty() {
                (
                    "No images match your search",
                    "Search by tag, digest, or image ID.",
                )
            } else {
                (
                    "No images on this Docker context",
                    "Images appear here after pulling or building them with Docker.",
                )
            };
            return empty_state(title, description).into_any_element();
        }
        let list = uniform_list(
            "images-scroll",
            self.images.visible.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                this.images.rows_rendered.set(range.len());
                range
                    .filter_map(|row| {
                        this.images
                            .visible
                            .get(row)
                            .and_then(|index| this.images.items.get(*index))
                    })
                    .map(|image| {
                        let selected = this.images.selection.contains(&image.id);
                        let id = image.id.clone();
                        ListItem::new(SharedString::from(format!(
                            "image-row-{}",
                            image.short_id()
                        )))
                        .selected(selected)
                        .aria_selected(selected)
                        .accessibility_label(image.name().to_owned())
                        .h(px(60.0))
                        .w_full()
                        .flex()
                        .items_center()
                        .px_4()
                        .gap_3()
                        .bg(rgb(if selected {
                            colors.selected
                        } else {
                            colors.background
                        }))
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.modifiers().control {
                                this.toggle_image_selection(id.clone(), window, cx);
                            } else {
                                this.select_image(id.clone(), window, cx);
                            }
                        }))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(div().text_ellipsis().child(image.name().to_owned()))
                                        .child(
                                            div().text_xs().text_color(rgb(colors.muted)).child(
                                                format!(
                                                    "{} · {} tag{}",
                                                    image.short_id(),
                                                    image.tags.len(),
                                                    if image.tags.len() == 1 { "" } else { "s" }
                                                ),
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "image-age-{}",
                                            image.short_id()
                                        )))
                                        .test_support()
                                        .w(px(140.0))
                                        .flex_shrink_0()
                                        .text_sm()
                                        .child(image.age_label()),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "image-size-{}",
                                            image.short_id()
                                        )))
                                        .test_support()
                                        .w(px(115.0))
                                        .flex_shrink_0()
                                        .child(image.size_label()),
                                )
                                .child(div().w(px(125.0)).flex_shrink_0().child(
                                    Tag::secondary().small().outline().child(
                                        if image.containers.is_empty() {
                                            "No containers".into()
                                        } else {
                                            format!(
                                                "{} container{}",
                                                image.containers.len(),
                                                if image.containers.len() == 1 { "" } else { "s" }
                                            )
                                        },
                                    ),
                                )),
                        )
                        .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.images.scroll)
        .size_full();
        div()
            .id("images-viewport")
            .test_support()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(list)
            .child(
                Scrollbar::vertical(&self.images.scroll)
                    .id("images-scrollbar")
                    .mode(ScrollbarMode::Always),
            )
            .into_any_element()
    }

    fn image_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let Some(image) = self.images.selected() else {
            return empty_state(
                "Select an image",
                "Choose an image to see its tags and container references.",
            )
            .into_any_element();
        };
        let detail = div()
            .id("image-detail")
            .test_support()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors.panel))
            .child(
                Toolbar::new("image-detail-toolbar")
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .content(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::MEDIUM)
                            .text_ellipsis()
                            .child(image.name().to_owned()),
                    )
                    .child(
                        Self::button("image-copy-id", "Copy ID", false, false, cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                if let Some(image) = this.images.selected() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        image.id.clone(),
                                    ));
                                    Self::notify("Image ID copied", false, window, cx);
                                }
                            }),
                        ),
                    ),
            );
        let detail = detail.child(self.image_tabs(cx));
        let content = match self.images.details.tab {
            ImageTab::Overview => self.image_overview(image, cx),
            ImageTab::History => self.image_history_view(cx),
            ImageTab::Inspect => self.image_inspect_view(),
            ImageTab::Analysis => self.image_analysis_view(cx),
        };
        detail
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }

    fn image_overview(&self, image: &Image, cx: &mut Context<Self>) -> AnyElement {
        let colors = palette(cx);
        let references = image.containers.clone();
        div().id("image-detail-scroll").size_full().overflow_y_scroll().px_5().py_3().flex().flex_col().gap_3()
            .child(div().id("image-full-id").test_support().text_xs().text_color(rgb(colors.muted)).child(image.id.clone()))
            .child(div().text_sm().child(format!("Size: {} · Created: {}", image.size_label(), image.created_label())))
            .child(div().text_xs().text_color(rgb(colors.muted)).child("Image size includes shared layers; it is not the space recovered by removal."))
            .child(div().id("image-tags").max_h(px(72.0)).overflow_y_scroll().flex().flex_wrap().gap_2().children(
                if image.tags.is_empty() { vec![Tag::secondary().small().outline().child("Untagged")] } else { image.tags.iter().map(|tag| Tag::secondary().small().outline().child(tag.clone())).collect() }))
            .child(div().font_weight(FontWeight::MEDIUM).child(format!("Containers using this image ({})", references.len())))
            .when(references.is_empty(), |view| view.child(div().text_color(rgb(colors.muted)).child("No containers reference this image. References are checked again before removal.")))
            .when(!references.is_empty(), |view| view.child(uniform_list("image-containers", references.len(), cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range.filter_map(|index| references.get(index)).map(|container| {
                    let id = container.id.clone();
                    let stopped = container.state != "running";
                    ListItem::new(SharedString::from(format!("image-container-{}", container.id))).h(px(36.0)).flex().items_center().gap_3().px_2()
                        .child(div().w_full().flex().items_center().gap_3()
                        .child(div().flex_1().text_ellipsis().child(container.name.clone()))
                        .child(Tag::secondary().small().outline().child(container.state.clone())))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !this.containers.items.iter().any(|container| container.id == id) { this.refresh(); return; }
                            this.show_images(false, window, cx);
                            if stopped { this.containers.running_only = false; }
                            this.containers.search_input.update(cx, |input, cx| input.set_value("", window, cx));
                            this.select(id.clone(), window, cx);
                            this.focus.focus(window, cx);
                        })).into_any_element()
                }).collect::<Vec<_>>()
            })).h(px(108.0)).flex_shrink_0()))
            .when(!image.containers.is_empty(), |view| view.child(div().text_color(rgb(colors.muted)).text_sm().child("Removal is unavailable while any running or stopped container uses this image.")))
            .into_any_element()
    }

    pub(super) fn images_workspace(&self, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let visible_selected = self
            .images
            .visible
            .iter()
            .filter(|index| {
                self.images
                    .selection
                    .contains(&self.images.items[**index].id)
            })
            .count();
        let selected_count = self.images.selection.len();
        let hidden = selected_count - visible_selected;
        let busy = !self.images.busy.is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                Toolbar::new("images-toolbar")
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .content(div().font_weight(FontWeight::MEDIUM).child("Images"))
                    .content(Tag::secondary().small().outline().child(format!(
                        "{} / {} images",
                        self.images.visible.len(),
                        self.images.items.len()
                    )))
                    .when(selected_count > 0, |toolbar| toolbar.content(Tag::secondary().small().outline().child(format!("{selected_count} selected{}", if hidden > 0 { format!(" · {hidden} hidden") } else { String::new() }))))
                    .when(self.images.loading, |toolbar| {
                        toolbar.content(Spinner::new().small())
                    })
                    .content(div().flex_1())
                    .child(Self::button("image-prune", if self.images.pruning.is_some() { "Pruning…" } else { "Prune…" }, false, !self.image_prune_available(), cx)
                        .loading(self.images.pruning.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.review_image_prune(window, cx))))
                    .content(
                        div()
                            .id("image-search")
                            .test_support()
                            .w(px(300.0))
                            .capture_key_down(cx.listener(Self::search_key))
                            .child(Input::new(&self.images.search).small()),
                    ),
            )
            .child(Toolbar::new("image-selection-toolbar").px_5().py_2()
                .content(div().flex_1().text_xs().text_color(rgb(colors.muted)).child("Ctrl-click to add or remove images from the selection. A normal click selects one."))
                .child(Self::button("image-remove", if busy { format!("Removing {} images…", self.images.busy.len()) } else if selected_count > 1 { format!("Remove selected ({selected_count})…") } else { "Remove image…".into() }, false, !self.image_removal_available(), cx)
                    .loading(busy).text_color(rgb(colors.error))
                    .on_click(cx.listener(|this, _, window, cx| this.review_image_removal(window, cx))))
            )
            .when_some(self.connection_error.clone(), |view, error| {
                view.child(Alert::error(
                    "images-connection-error",
                    format!("{error} · reconnecting…"),
                ))
            })
            .when_some(self.images.error.clone(), |view, error| {
                view.child(Alert::error("images-error", error))
            })
            .child(
                div().flex_1().min_h_0().child(v_resizable("images-split")
                    .with_state(&self.images.dock)
                    .child(
                        resizable_panel().size_range(px(80.0)..Pixels::MAX).child(
                            div()
                                .size_full()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .flex()
                                        .gap_3()
                                        .px_4()
                                        .py_1()
                                        .text_xs()
                                        .text_color(rgb(colors.muted))
                                        .child(
                                            div()
                                                .flex_1()
                                                .flex()
                                                .child(self.image_sort_header(ImageSort::Name, cx)),
                                        )
                                        .child(div().w(px(140.0)).child(self.image_sort_header(ImageSort::Age, cx)))
                                        .child(
                                            div()
                                                .w(px(115.0))
                                                .child(self.image_sort_header(ImageSort::Size, cx)),
                                        )
                                        .child(div().w(px(125.0)).child(
                                            self.image_sort_header(ImageSort::References, cx),
                                        )),
                                )
                                .child(
                                    div()
                                        .id("images-pane")
                                        .test_support()
                                        .flex_1()
                                        .min_h_0()
                                        .child(self.image_list(cx)),
                                ),
                        ),
                    )
                    .child(
                        resizable_panel()
                            .size(px(310.0))
                            .size_range(px(260.0)..Pixels::MAX)
                            .child(self.image_detail(cx)),
                    )),
            )
    }
}
