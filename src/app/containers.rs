//! Container discovery, selection, controls, and detail navigation.
use super::FerryDock;
use super::Tab;
use super::ui::FeedbackNotification;
use super::ui::{empty_state, palette};
use crate::ansi::Buffer as StyledLogBuffer;
use crate::docker::Action;
use crate::docker::Command;
use crate::model::Container;
use gpui::{prelude::*, *};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::Editor;
use gpui_kit::component::input::EditorState;
use gpui_kit::component::input::InputState;
use gpui_kit::component::tab::Tab as KitTab;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{menu::DropdownMenu, toolbar::Toolbar};
use std::collections::HashMap;

pub(super) struct Containers {
    pub(super) items: Vec<Container>,
    pub(super) selected: Option<String>,
    pub(super) inspection: Entity<EditorState>,
    pub(super) busy: HashMap<String, Action>,
    pub(super) running_only: bool,
    pub(super) filter: String,
    pub(super) search_input: Entity<InputState>,
    pub(super) search_focus: FocusHandle,
    pub(super) scroll: UniformListScrollHandle,
}

impl Containers {
    pub(super) fn new(
        search_input: Entity<InputState>,
        search_focus: FocusHandle,
        inspection: Entity<EditorState>,
    ) -> Self {
        Self {
            items: Vec::new(),
            selected: None,
            inspection,
            busy: HashMap::new(),
            running_only: true,
            filter: String::new(),
            search_input,
            search_focus,
            scroll: UniformListScrollHandle::default(),
        }
    }
}

impl FerryDock {
    pub(super) fn selected_container(&self) -> Option<&Container> {
        self.containers.selected.as_ref().and_then(|id| {
            self.containers
                .items
                .iter()
                .find(|container| &container.id == id)
        })
    }

    pub(super) fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let returning_from_project = self.projects.selected.is_some();
        if self.projects.selected.is_some() {
            self.backend.send(Command::CloseProjectLogs);
            self.project_logs.generation += 1;
        }
        self.projects.selected = None;
        self.projects.form = None;
        if self.tab == Tab::Compose {
            self.tab = Tab::Logs;
        }
        if self.containers.selected.as_ref() == Some(&id) && !returning_from_project {
            return;
        }
        if self.containers.selected.as_ref() != Some(&id) {
            self.backend.send(Command::CloseTerminal);
            self.terminal.view = None;
        }
        self.containers.selected = Some(id.clone());
        self.container_logs.generation += 1;
        self.container_logs.buffer = StyledLogBuffer::with_timestamps();
        self.container_logs.scroll = UniformListScrollHandle::default();
        self.containers.inspection.update(cx, |editor, cx| {
            editor.set_value("Loading container configuration…", window, cx)
        });
        self.container_logs.status = "Connecting to logs…".into();
        self.container_logs.follow = true;
        self.backend.send(Command::Logs {
            id: id.clone(),
            generation: self.container_logs.generation,
        });
        self.backend.send(Command::Inspect {
            id,
            generation: self.container_logs.generation,
        });
        // Avoid opening another interactive process merely by changing rows.
        if self.tab == Tab::Terminal {
            self.tab = Tab::Logs;
        }
        cx.notify();
    }

    pub(super) fn action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let Some(container) = self.selected_container() else {
            return;
        };
        if self.connection.is_none()
            || self.containers.busy.contains_key(&container.id)
            || container
                .project
                .as_ref()
                .is_some_and(|project| self.project_busy(project))
        {
            return;
        }
        let id = container.id.clone();
        if matches!(action, Action::Stop | Action::Restart) {
            self.backend.send(Command::CloseTerminal);
            if let Some(view) = &self.terminal.view {
                view.update(cx, |view, cx| {
                    view.ready = false;
                    view.status = "Container operation closed this shell session".into();
                    cx.notify();
                });
            }
        }
        self.containers.busy.insert(id.clone(), action);
        window.remove_notification::<FeedbackNotification>(cx);
        self.backend.send(Command::Action { id, action });
        cx.notify();
    }

    pub(super) fn detail(&self, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        if let Some(project) = &self.projects.selected {
            return self.project_detail(project, cx);
        }
        let Some(container) = self.selected_container() else {
            return div().size_full().child(empty_state(
                "Select a container",
                "View logs, open a shell, or inspect a container from the list above.",
            ));
        };
        let unavailable = self.connection.is_none()
            || self.containers.busy.contains_key(&container.id)
            || container
                .project
                .as_ref()
                .is_some_and(|project| self.project_busy(project));
        let primary = if container.running() {
            Action::Stop
        } else {
            Action::Start
        };
        let mut detail = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(colors.background))
            .border_t_1()
            .border_color(rgb(colors.border));
        detail =
            detail.child(
                Toolbar::new("container-detail-toolbar")
                    .gap_2()
                    .px_4()
                    .py_2()
                    .content(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(container.name.clone())
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(colors.muted))
                                    .child(format!(
                                        "{} · {}",
                                        container.project.as_deref().unwrap_or("Standalone"),
                                        &container.id[..container.id.len().min(12)]
                                    )),
                            ),
                    )
                    .child(
                        Self::button("primary-action", primary.label(), false, unavailable, cx)
                            .loading(self.containers.busy.contains_key(&container.id))
                            .when(self.containers.busy.contains_key(&container.id), |button| {
                                button.icon(IconName::Loader)
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.action(primary, window, cx)
                            })),
                    )
                    .child(
                        Self::button(
                            "restart",
                            "Restart",
                            false,
                            unavailable || !container.running(),
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.selected_container().is_some_and(Container::running) {
                                this.action(Action::Restart, window, cx);
                            }
                        })),
                    )
                    .content(
                        Self::button("container-more", "More", false, false, cx)
                            .icon(IconName::Ellipsis)
                            .tooltip("Container actions and copy commands")
                            .dropdown_menu(self.container_menu(container.id.clone(), cx)),
                    ),
            );
        let items = [Tab::Logs, Tab::Terminal, Tab::Inspect];
        let tabs = TabBar::new("container-tabs")
            .small()
            .underline()
            .px_4()
            .selected_index(items.iter().position(|tab| *tab == self.tab).unwrap_or(0))
            .children(items.map(|tab| KitTab::new().label(tab.name())))
            .on_click(cx.listener(move |this, index: &usize, window, cx| {
                this.switch_tab(items[*index], window, cx)
            }));
        detail.child(tabs).child(
            div().flex_1().min_h_0().child(match self.tab {
                Tab::Logs => self.logs_view(cx).into_any_element(),
                Tab::Terminal => self.terminal_view(cx).into_any_element(),
                Tab::Inspect => div()
                    .id("inspection")
                    .test_support()
                    .debug_selector(|| "inspection".to_string())
                    .size_full()
                    .child(
                        Editor::new(&self.containers.inspection)
                            .readonly(true)
                            .h_full(),
                    )
                    .into_any_element(),
                Tab::Compose => div().into_any_element(),
            }),
        )
    }
}
