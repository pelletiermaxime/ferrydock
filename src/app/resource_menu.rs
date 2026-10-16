//! Kit resource menus share the same guarded actions as the detail controls.
use super::{FerryDock, Tab};
use crate::compose;
use crate::docker::Action;
use gpui::*;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

impl FerryDock {
    pub(super) fn project_menu(
        &self,
        project: String,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.entity().downgrade();
        move |mut menu, _, cx| {
            let Some(view) = owner.upgrade() else {
                return menu;
            };
            let this = view.read(cx);
            let configured = this
                .projects
                .items
                .iter()
                .any(|item| item.name == project && item.config.is_some());
            let busy = this.project_busy(&project);
            let connected = this.connection.is_some();
            let saving = this.projects.form.as_ref().is_some_and(|form| form.saving);
            for (label, configure) in [
                ("View project logs", false),
                ("Configure Compose files", true),
            ] {
                let owner = owner.clone();
                let project = project.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .disabled(saving || (configure && (!connected || busy)))
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if this.projects.form.as_ref().is_some_and(|form| form.saving)
                                    || (configure
                                        && (this.connection.is_none()
                                            || this.project_busy(&project)))
                                {
                                    return;
                                }
                                this.select_project(project.clone(), window, cx);
                                if configure {
                                    this.configure_project(window, cx);
                                }
                            });
                        }),
                );
            }
            menu = menu.separator();
            for action in [
                compose::Action::Start,
                compose::Action::Stop,
                compose::Action::Restart,
                compose::Action::Rebuild,
            ] {
                let owner = owner.clone();
                let project = project.clone();
                menu = menu.item(
                    PopupMenuItem::new(action.label())
                        .disabled(!configured || !connected || busy || this.projects.form.is_some())
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if this.projects.form.is_some()
                                    || this.connection.is_none()
                                    || this.project_busy(&project)
                                {
                                    return;
                                }
                                this.select_project(project.clone(), window, cx);
                                this.compose_action(action, cx);
                            });
                        }),
                );
            }
            let owner = owner.clone();
            let project = project.clone();
            menu = menu.separator().item(
                PopupMenuItem::new("Stop & cleanup…")
                    .disabled(!configured || !connected || busy || this.projects.form.is_some())
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.review_project_cleanup(project.clone(), window, cx);
                        });
                    }),
            );
            menu
        }
    }

    pub(super) fn container_menu(
        &self,
        id: String,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.entity().downgrade();
        move |mut menu, _, cx| {
            let Some(view) = owner.upgrade() else {
                return menu;
            };
            let this = view.read(cx);
            let Some(container) = this
                .containers
                .items
                .iter()
                .find(|container| container.id == id)
            else {
                return menu;
            };
            let unavailable = this.container_action_unavailable(container);
            let running = container.running();
            for (label, tab) in [
                ("View logs", Tab::Logs),
                ("Open terminal", Tab::Terminal),
                ("Inspect", Tab::Inspect),
            ] {
                let owner = owner.clone();
                let id = id.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .disabled(tab == Tab::Terminal && (!running || unavailable))
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if !this
                                    .containers
                                    .items
                                    .iter()
                                    .any(|container| container.id == id)
                                {
                                    return;
                                }
                                this.select(id.clone(), window, cx);
                                this.switch_tab(tab, window, cx);
                            });
                        }),
                );
            }
            {
                let owner = owner.clone();
                let id = id.clone();
                menu = menu.item(
                    PopupMenuItem::new("View networks")
                        .disabled(this.connection.is_none())
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.show_container_networks(id.clone(), window, cx)
                            });
                        }),
                );
            }
            menu = menu.separator();
            for (label, value) in [
                ("Copy container name", container.name.clone()),
                ("Copy container ID", container.id.clone()),
            ] {
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
                }));
            }
            menu = menu.separator();
            for action in [
                if running { Action::Stop } else { Action::Start },
                Action::Restart,
            ] {
                let owner = owner.clone();
                let id = id.clone();
                menu = menu.item(
                    PopupMenuItem::new(action.label())
                        .disabled(unavailable || (matches!(action, Action::Restart) && !running))
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.container_action(&id, action, window, cx);
                            });
                        }),
                );
            }
            menu
        }
    }
}
