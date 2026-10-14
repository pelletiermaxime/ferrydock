//! Project-scoped teardown review, kept separate from non-destructive controls.
use super::{FerryDock, ui::palette};
use crate::compose::{Action, Config};
use gpui::{prelude::*, *};
use gpui_kit::{
    base::TestSupportExt,
    component::{WindowExt as _, checkbox::Checkbox, dialog::Dialog},
};

pub(super) struct CleanupReview {
    project: String,
    endpoint: String,
    config: Config,
    containers: Vec<(String, String)>,
    volumes: bool,
    images: bool,
}

impl FerryDock {
    fn cleanup_members(&self, project: &str) -> Vec<(String, String)> {
        let mut members = self
            .containers
            .items
            .iter()
            .filter(|container| container.project.as_deref() == Some(project))
            .map(|container| (container.id.clone(), container.name.clone()))
            .collect::<Vec<_>>();
        members.sort();
        members
    }

    pub(super) fn cleanup_review_available(&self) -> bool {
        let Some(review) = &self.projects.cleanup else {
            return false;
        };
        self.connection
            .as_ref()
            .is_some_and(|connection| connection.endpoint == review.endpoint)
            && self.projects.selected.as_ref() == Some(&review.project)
            && !self.project_busy(&review.project)
            && self.projects.form.is_none()
            && self.projects.items.iter().any(|project| {
                project.name == review.project && project.config.as_ref() == Some(&review.config)
            })
            && self.cleanup_members(&review.project) == review.containers
    }

    pub(super) fn review_project_cleanup(
        &mut self,
        project: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.project_busy(&project)
            || self.projects.form.is_some()
            || self.projects.cleanup.is_some()
        {
            return;
        }
        let Some(connection) = &self.connection else {
            return;
        };
        let Some(config) = self
            .projects
            .items
            .iter()
            .find(|item| item.name == project)
            .and_then(|item| item.config.clone())
        else {
            return;
        };
        let endpoint = connection.endpoint.clone();
        let containers = self.cleanup_members(&project);
        self.select_project(project.clone(), window, cx);
        self.projects.cleanup = Some(CleanupReview {
            project,
            endpoint,
            config,
            containers,
            volumes: true,
            images: true,
        });
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            owner
                .update(cx, |this, cx| this.project_cleanup_dialog(dialog, cx))
                .unwrap_or_else(|_| Dialog::new(cx).title("Project unavailable"))
        });
        cx.notify();
    }

    pub(super) fn invalidate_cleanup_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.projects.cleanup.is_some() && !self.cleanup_review_available() {
            self.projects.cleanup = None;
            window.close_dialog(cx);
        }
    }

    fn cancel_project_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.projects.cleanup = None;
        window.close_dialog(cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn confirm_project_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.cleanup_review_available() {
            return false;
        }
        let review = self.projects.cleanup.as_ref().unwrap();
        let action = Action::Cleanup {
            volumes: review.volumes,
            images: review.images,
        };
        self.compose_action(action, cx);
        self.projects.cleanup = None;
        self.focus.focus(window, cx);
        true
    }

    fn project_cleanup_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let Some(review) = &self.projects.cleanup else {
            return dialog.title("Project unavailable");
        };
        let colors = palette(cx);
        let targets = review.containers.clone();
        let owner = cx.entity().downgrade();
        let confirm = owner.clone();
        dialog.title(format!("Stop & cleanup {}?", review.project)).width(px(640.0)).overlay_closable(false)
            .on_close(cx.listener(|this, _, _, cx| { this.projects.cleanup = None; cx.notify(); }))
            .on_cancel(move |_, window, cx| owner.update(cx, |this, cx| { this.projects.cleanup = None; this.focus.focus(window, cx); cx.notify(); true }).unwrap_or(true))
            .on_ok(move |_, window, cx| confirm.update(cx, |this, cx| this.confirm_project_cleanup(window, cx)).unwrap_or(false))
            .child(div().flex().flex_col().gap_3()
                .child("Stop and remove every container in this Compose project, including stopped services and orphans, and remove its networks.")
                .child(div().text_sm().text_color(rgb(colors.muted)).child(review.config.directory.display().to_string()))
                .when(!targets.is_empty(), |body| body.child(uniform_list("compose-cleanup-targets", targets.len(), cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                    range.filter_map(|index| targets.get(index)).map(|(id, name)| div().id(SharedString::from(format!("compose-cleanup-target-{id}"))).test_support().h(px(28.0)).text_ellipsis().child(name.clone()).into_any_element()).collect::<Vec<_>>()
                })).h(px(28.0 * review.containers.len().clamp(1, 4) as f32))))
                .child(Checkbox::new("compose-cleanup-volumes").debug_selector(|| "compose-cleanup-volumes".into())
                    .label("Delete project volumes and their data permanently").checked(review.volumes)
                    .on_click(cx.listener(|this, checked, _, cx| { if let Some(review) = &mut this.projects.cleanup { review.volumes = *checked; } cx.notify(); })))
                .child(Checkbox::new("compose-cleanup-images").debug_selector(|| "compose-cleanup-images".into())
                    .label("Remove Compose-generated local images").checked(review.images)
                    .on_click(cx.listener(|this, checked, _, cx| { if let Some(review) = &mut this.projects.cleanup { review.images = *checked; } cx.notify(); })))
                .child(div().text_sm().text_color(rgb(colors.muted)).child("Volume deletion cannot be undone. External volumes and networks, bind-mounted files, explicitly tagged images and build cache are kept. The saved Compose association is removed after successful cleanup.")))
            .footer(div().flex().justify_end().gap_2()
                .child(Self::button("compose-cleanup-cancel", "Cancel", false, false, cx)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_project_cleanup(window, cx))))
                .child(Self::button("compose-cleanup-confirm", "Stop & cleanup", false, !self.cleanup_review_available(), cx).text_color(rgb(colors.error))
                    .on_click(cx.listener(|this, _, window, cx| { if this.confirm_project_cleanup(window, cx) { window.close_dialog(cx); } }))))
    }
}
