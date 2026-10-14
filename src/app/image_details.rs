//! Selected image history and inspection, separate from inventory operations.
use std::sync::Arc;

use bollard::models::ImageHistoryResponseItem;
use gpui::{prelude::*, *};
use gpui_kit::{
    base::TestSupportExt,
    component::{
        Sizable, WindowExt,
        alert::Alert,
        input::{Editor, EditorState},
        tab::{Tab, TabBar},
    },
};

use super::{FerryDock, image_history::ImageHistory, ui::empty_state};
use crate::docker::{Command, Event};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ImageTab {
    #[default]
    Overview,
    History,
    Inspect,
    Analysis,
}

pub(super) struct ImageDetails {
    pub(super) tab: ImageTab,
    pub(super) id: Option<String>,
    pub(super) generation: u64,
    pub(super) requested: bool,
    pub(super) history: Option<Result<Arc<[ImageHistoryResponseItem]>, String>>,
    pub(super) inspection: Option<Result<(), String>>,
    pub(super) editor: Entity<EditorState>,
    pub(super) analysis: super::image_analysis::Analysis,
    pub(super) history_view: ImageHistory,
}

impl ImageDetails {
    pub(super) fn new(window: &mut Window, cx: &mut Context<FerryDock>) -> Self {
        Self {
            tab: ImageTab::Overview,
            id: None,
            generation: 0,
            requested: false,
            history: None,
            inspection: None,
            editor: cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("json")
                    .folding(true)
                    .soft_wrap(false)
            }),
            history_view: ImageHistory::new(window, cx),
            analysis: super::image_analysis::Analysis::new(cx),
        }
    }

    pub(super) fn reset(&mut self) {
        self.analysis.invalidate();
        self.generation += 1;
        self.id = None;
        self.requested = false;
        self.history = None;
        self.inspection = None;
    }
}

impl FerryDock {
    pub(super) fn invalidate_image_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.images.details.requested {
            self.backend.send(Command::CloseImageDetails);
        }
        if self.images.details.analysis.loading {
            self.backend.send(Command::CloseImageAnalysis);
        }
        self.images.details.analysis.reset(cx);
        self.images.details.reset();
        self.images
            .details
            .editor
            .update(cx, |editor, cx| editor.set_value("", window, cx));
        if self.images.details.history_view.command_open {
            self.images.details.history_view.command_open = false;
            window.close_dialog(cx);
        }
        self.images.details.history_view.set_rows(Arc::from([]), cx);
    }

    pub(super) fn sync_image_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.images.selected.clone();
        if self.images.details.id != id {
            self.invalidate_image_details(window, cx);
            self.images.details.id = id.clone();
        }
        if self.images_open
            && self.connection.is_some()
            && matches!(
                self.images.details.tab,
                ImageTab::History | ImageTab::Inspect
            )
            && !self.images.details.requested
            && let Some(id) = id
        {
            self.images.details.requested = true;
            self.backend.send(Command::ImageDetails {
                id,
                generation: self.images.details.generation,
            });
        }
    }

    pub(super) fn switch_image_tab(
        &mut self,
        tab: ImageTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.images.details.tab = tab;
        self.sync_image_details(window, cx);
        if tab == ImageTab::Inspect
            && self
                .images
                .details
                .inspection
                .as_ref()
                .is_none_or(Result::is_ok)
            && self.connection.is_some()
            && self.images.details.requested
        {
            self.images
                .details
                .editor
                .focus_handle(cx)
                .focus(window, cx);
        } else if tab == ImageTab::History
            && self
                .images
                .details
                .history
                .as_ref()
                .is_some_and(Result::is_ok)
            && self.connection.is_some()
        {
            self.images
                .details
                .history_view
                .table
                .focus_handle(cx)
                .focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn image_details_event(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.images_open || self.connection.is_none() {
            return;
        }
        let details = &mut self.images.details;
        match event {
            Event::ImageHistory {
                id,
                generation,
                result,
            } if generation == details.generation
                && details.id.as_ref() == Some(&id)
                && details.requested =>
            {
                let result = result.map(Arc::<[ImageHistoryResponseItem]>::from);
                match &result {
                    Ok(rows) => {
                        details.history_view.set_rows(rows.clone(), cx);
                        if details.tab == ImageTab::History && self.focus.is_focused(window) {
                            details
                                .history_view
                                .table
                                .focus_handle(cx)
                                .focus(window, cx);
                        }
                    }
                    Err(_)
                        if details.tab == ImageTab::History
                            && details
                                .history_view
                                .table
                                .focus_handle(cx)
                                .contains_focused(window, cx) =>
                    {
                        self.focus.focus(window, cx);
                    }
                    Err(_) => {}
                }
                details.history = Some(result);
            }
            Event::ImageInspection {
                id,
                generation,
                result,
            } if generation == details.generation
                && details.id.as_ref() == Some(&id)
                && details.requested =>
            {
                details.inspection = Some(match result {
                    Ok(text) => {
                        details
                            .editor
                            .update(cx, |editor, cx| editor.set_value(text, window, cx));
                        Ok(())
                    }
                    Err(error) => {
                        if details.tab == ImageTab::Inspect
                            && details.editor.focus_handle(cx).contains_focused(window, cx)
                        {
                            self.focus.focus(window, cx);
                        }
                        Err(error)
                    }
                });
            }
            _ => {}
        }
    }

    pub(super) fn image_tabs(&self, cx: &mut Context<Self>) -> TabBar {
        let tabs = [
            ImageTab::Overview,
            ImageTab::History,
            ImageTab::Inspect,
            ImageTab::Analysis,
        ];
        TabBar::new("image-tabs")
            .small()
            .underline()
            .px_4()
            .selected_index(
                tabs.iter()
                    .position(|tab| *tab == self.images.details.tab)
                    .unwrap_or(0),
            )
            .children(
                ["Overview", "History", "Inspect", "Analysis"].map(|name| Tab::new().label(name)),
            )
            .on_click(cx.listener(move |this, index: &usize, window, cx| {
                this.switch_image_tab(tabs[*index], window, cx);
            }))
    }

    pub(super) fn image_history_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let details = &self.images.details;
        if self.connection.is_none() {
            return empty_state(
                "History unavailable",
                "Reconnect to Docker to load image history.",
            )
            .into_any_element();
        }
        let rows = match &details.history {
            None => {
                return empty_state("Loading history…", "Reading image layers from Docker.")
                    .into_any_element();
            }
            Some(Err(error)) => {
                return Alert::error(
                    "image-history-error",
                    format!("History failed: {error} · Refresh to retry."),
                )
                .into_any_element();
            }
            Some(Ok(rows)) if rows.is_empty() => {
                return empty_state(
                    "No image history",
                    "Docker returned no history entries for this image.",
                )
                .into_any_element();
            }
            Some(Ok(rows)) => rows.clone(),
        };
        self.compact_history(rows.len(), cx)
    }

    pub(super) fn image_inspect_view(&self) -> AnyElement {
        if self.connection.is_none() {
            return empty_state(
                "Inspection unavailable",
                "Reconnect to Docker to inspect this image.",
            )
            .into_any_element();
        }
        if !self.images.details.requested {
            return empty_state(
                "Loading image inspection…",
                "Refreshing image details from Docker.",
            )
            .into_any_element();
        }
        match &self.images.details.inspection {
            Some(Err(error)) => Alert::error(
                "image-inspect-error",
                format!("Inspection failed: {error} · Refresh to retry."),
            )
            .into_any_element(),
            // Keep the editor mounted and focused while its async read completes.
            _ => div()
                .id("image-inspection")
                .test_support()
                .size_full()
                .flex()
                .flex_col()
                .when(self.images.details.inspection.is_none(), |view| {
                    view.child(div().px_5().py_2().child("Loading image inspection…"))
                })
                .child(
                    div().flex_1().min_h_0().child(
                        Editor::new(&self.images.details.editor)
                            .readonly(true)
                            .h_full(),
                    ),
                )
                .into_any_element(),
        }
    }
}
