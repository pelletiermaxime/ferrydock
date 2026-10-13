//! Compose association forms, operations, and project controls.
use super::FerryDock;
use super::ui::empty_state;
use super::ui::palette;
use crate::ansi::Buffer as StyledLogBuffer;
use crate::compose;
use crate::compose::Config as ComposeConfig;
use crate::compose::Project as ComposeProject;
use crate::docker::Command;
use gpui::{prelude::*, *};
use gpui_kit::base::FocusTrapElement;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Escape as InputEscape;
use gpui_kit::component::input::IndentInline;
use gpui_kit::component::input::Input;
use gpui_kit::component::input::InputState;
use gpui_kit::component::input::OutdentInline;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::input::TextareaState;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tab::Tab as KitTab;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{
    alert::Alert,
    description_list::{DescriptionItem, DescriptionList},
    label::Label,
    scroll::{Scrollbar, ScrollbarMode},
    spinner::Spinner,
    tag::Tag,
    toolbar::Toolbar,
};
use std::collections::HashMap;

pub(super) struct ComposeForm {
    pub(super) project: String,
    pub(super) directory: Entity<InputState>,
    pub(super) lists: [Entity<TextareaState>; 3],
    pub(super) saving: bool,
    pub(super) error: Option<String>,
}

pub(super) struct ComposeOperation {
    pub(super) id: u64,
    pub(super) action: compose::Action,
    pub(super) running: bool,
    pub(super) output: StyledLogBuffer,
    pub(super) output_scroll: UniformListScrollHandle,
    pub(super) result: Option<Result<(), String>>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ProjectTab {
    Logs,
    Operations,
}

pub(super) struct Projects {
    pub(super) items: Vec<ComposeProject>,
    pub(super) selected: Option<String>,
    pub(super) form: Option<ComposeForm>,
    pub(super) focus: FocusHandle,
    pub(super) operations: HashMap<String, ComposeOperation>,
    pub(super) next_operation: u64,
    pub(super) tab: ProjectTab,
}
impl Projects {
    pub(super) fn new(focus: FocusHandle) -> Self {
        Self {
            items: Vec::new(),
            selected: None,
            form: None,
            focus,
            operations: HashMap::new(),
            next_operation: 0,
            tab: ProjectTab::Logs,
        }
    }
}

impl FerryDock {
    pub(super) fn project_busy(&self, project: &str) -> bool {
        self.projects
            .operations
            .get(project)
            .is_some_and(|operation| operation.running)
            || self.containers.items.iter().any(|container| {
                container.project.as_deref() == Some(project)
                    && self.containers.busy.contains_key(&container.id)
            })
    }

    pub(super) fn configure_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self
            .projects
            .items
            .iter()
            .find(|project| Some(&project.name) == self.projects.selected.as_ref())
        else {
            return;
        };
        if self.project_busy(&project.name) {
            return;
        }
        let config = project.config.as_ref().unwrap_or(&project.hint);
        let paths = |paths: &[std::path::PathBuf]| {
            paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.projects.form = Some(ComposeForm {
            project: project.name.clone(),
            directory: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(config.directory.to_string_lossy().into_owned())
            }),
            lists: [
                paths(&config.files),
                paths(&config.env_files),
                config.profiles.join("\n"),
            ]
            .map(|value| cx.new(|cx| TextareaState::new(window, cx).rows(2).default_value(value))),
            saving: false,
            error: None,
        });
        self.expand_dock(500.0, window, cx);
        self.projects
            .form
            .as_ref()
            .unwrap()
            .directory
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    pub(super) fn save_project(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.projects.form else {
            return;
        };
        if form.saving || self.connection.is_none() || self.project_busy(&form.project) {
            return;
        }
        let paths = |text: &str| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(std::path::PathBuf::from)
                .collect()
        };
        let config = ComposeConfig {
            project: form.project.clone(),
            directory: form.directory.read(cx).value().to_string().into(),
            files: paths(&form.lists[0].read(cx).value()),
            env_files: paths(&form.lists[1].read(cx).value()),
            profiles: form.lists[2]
                .read(cx)
                .value()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect(),
        };
        let form = self.projects.form.as_mut().unwrap();
        form.saving = true;
        form.error = None;
        self.backend.send(Command::ConfigureCompose { config });
        cx.notify();
    }

    pub(super) fn compose_action(&mut self, action: compose::Action, cx: &mut Context<Self>) {
        let Some(project) = self.projects.selected.clone() else {
            return;
        };
        if self.connection.is_none()
            || self.project_busy(&project)
            || self.projects.form.is_some()
            || !self
                .projects
                .items
                .iter()
                .any(|item| item.name == project && item.config.is_some())
        {
            return;
        }
        self.projects.next_operation += 1;
        self.projects.tab = ProjectTab::Operations;
        self.projects.operations.insert(
            project.clone(),
            ComposeOperation {
                id: self.projects.next_operation,
                action,
                running: true,
                output: StyledLogBuffer::default(),
                output_scroll: UniformListScrollHandle::default(),
                result: None,
            },
        );
        if matches!(action, compose::Action::Stop | compose::Action::Restart)
            && self
                .selected_container()
                .is_some_and(|container| container.project.as_ref() == Some(&project))
        {
            self.backend.send(Command::CloseTerminal);
            if let Some(terminal) = &self.terminal.view {
                terminal.update(cx, |view, cx| {
                    view.ready = false;
                    view.status = "Compose operation closed this shell session".into();
                    cx.notify();
                });
            }
        }
        self.backend.send(Command::Compose {
            project,
            operation: self.projects.next_operation,
            action,
        });
        cx.notify();
    }

    pub(super) fn project_detail(&self, name: &str, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let project = self
            .projects
            .items
            .iter()
            .find(|project| project.name == name);
        let configured = project.is_some_and(|project| project.config.is_some());
        let busy = self.project_busy(name);
        let disabled =
            !configured || busy || self.connection.is_none() || self.projects.form.is_some();
        let total = self
            .containers
            .items
            .iter()
            .filter(|container| container.project.as_deref() == Some(name))
            .count();
        let running = self
            .containers
            .items
            .iter()
            .filter(|container| container.project.as_deref() == Some(name) && container.running())
            .count();
        let mut toolbar = Toolbar::new("compose-toolbar").gap_2().px_4().py_2().border_b_1().border_color(rgb(colors.border))
            .content(div().flex_1().min_w_0().child(format!("Compose · {name}"))
                .child(div().text_size(px(11.0)).text_color(rgb(colors.muted)).child(format!("{running} running / {total} containers · actions apply to the configured project and profiles"))))
            .child(Self::button("compose-configure", if configured { "Configuration" } else { "Associate files" }, false, busy || self.connection.is_none(), cx)
                .on_click(cx.listener(|this, _, window, cx| { if this.connection.is_some() { this.configure_project(window, cx); } })));
        for (action, id) in [
            (compose::Action::Start, "compose-start"),
            (compose::Action::Stop, "compose-stop"),
            (compose::Action::Restart, "compose-restart"),
        ] {
            let loading = self
                .projects
                .operations
                .get(name)
                .is_some_and(|operation| operation.running && operation.action == action);
            toolbar = toolbar.child(
                Self::button(id, action.label(), false, disabled, cx)
                    .loading(loading)
                    .when(loading, |button| button.icon(IconName::Loader))
                    .on_click(cx.listener(move |this, _, _, cx| this.compose_action(action, cx))),
            );
        }
        toolbar = toolbar.content(
            Self::button("project-more", "More", false, false, cx)
                .icon(IconName::Ellipsis)
                .tooltip("Project logs, configuration, and actions")
                .dropdown_menu(self.project_menu(name.to_owned(), cx)),
        );
        let mut pane = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(colors.background))
            .border_t_1()
            .border_color(rgb(colors.border))
            .child(toolbar);
        if let Some(form) = &self.projects.form {
            let controls = Toolbar::new("compose-form-toolbar")
                .flex()
                .gap_2()
                .px_4()
                .py_2()
                .child(
                    Self::button(
                        "compose-save",
                        if form.saving {
                            "Saving…"
                        } else {
                            "Save association"
                        },
                        true,
                        form.saving || self.connection.is_none(),
                        cx,
                    )
                    .loading(form.saving)
                    .when(form.saving, |button| button.icon(IconName::Loader))
                    .on_click(cx.listener(|this, _, _, cx| this.save_project(cx))),
                )
                .child(
                    Self::button("compose-cancel-config", "Cancel", false, form.saving, cx)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.projects.form.as_ref().is_some_and(|form| form.saving) {
                                this.projects.form = None;
                                this.focus.focus(window, cx);
                                cx.notify();
                            }
                        })),
                );
            pane = pane.child(controls);
            if let Some(error) = &form.error {
                pane = pane.child(
                    Alert::error("compose-form-error", error.clone())
                        .title("Association could not be saved"),
                );
            }
            let mut fields = div().id("compose-form").flex_1().min_h_0().overflow_y_scroll().p_4().flex().flex_col().gap_2()
                .track_focus(&self.projects.focus)
                .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                    window.focus_next(cx);
                    if !this.projects.focus.contains_focused(window, cx)
                        && let Some(form) = &this.projects.form
                    {
                        form.directory.focus_handle(cx).focus(window, cx);
                    }
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &OutdentInline, window, cx| {
                    window.focus_prev(cx);
                    if !this.projects.focus.contains_focused(window, cx)
                        && let Some(form) = &this.projects.form
                    {
                        form.lists[2].focus_handle(cx).focus(window, cx);
                    }
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &InputEscape, window, cx| {
                    if !this.projects.form.as_ref().is_some_and(|form| form.saving) {
                        this.projects.form = None;
                        this.focus.focus(window, cx);
                        cx.notify();
                    }
                    cx.stop_propagation();
                }))
                .child(div().text_size(px(11.0)).text_color(rgb(colors.muted)).child("Review label suggestions. Tab: next field · Ctrl+A: select all · Ctrl+V: paste · Escape: cancel"));
            for (index, label) in [
                "Project directory (absolute path)",
                "Compose files (one per line, in override order)",
                "Environment files (one per line; empty uses Compose's default .env)",
                "Profiles (one per line; empty means no explicit profiles)",
            ]
            .into_iter()
            .enumerate()
            {
                let input = if index == 0 {
                    Input::new(&form.directory)
                        .small()
                        .disabled(form.saving)
                        .aria_label(label)
                        .into_any_element()
                } else {
                    Textarea::new(&form.lists[index - 1])
                        .small()
                        .disabled(form.saving)
                        .aria_label(label)
                        .into_any_element()
                };
                fields = fields.child(Label::new(label).text_size(px(12.0))).child(
                    div()
                        .id(SharedString::from(format!("compose-field-{index}")))
                        .test_support()
                        .debug_selector(move || format!("compose-field-{index}"))
                        .text_size(px(12.0))
                        .font_family(super::ui::MONOSPACE_FONT)
                        .child(input),
                );
            }
            return pane.child(fields.focus_trap("compose-fields", &self.projects.focus));
        }
        pane = pane.child(
            TabBar::new("project-tabs")
                .small()
                .segmented()
                .selected_index(usize::from(self.projects.tab == ProjectTab::Operations))
                .children([
                    KitTab::new().label("Logs"),
                    KitTab::new().label("Operations"),
                ])
                .on_click(cx.listener(|this, index: &usize, _, cx| {
                    this.projects.tab = if *index == 0 {
                        ProjectTab::Logs
                    } else {
                        ProjectTab::Operations
                    };
                    cx.notify();
                })),
        );
        if self.projects.tab == ProjectTab::Logs {
            return pane.child(self.project_logs_view(name, cx));
        }
        let mut body = div()
            .id("compose-output")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(operation) = self.projects.operations.get(name) {
            let status = if operation.running {
                format!("{} in progress…", operation.action.label())
            } else {
                match &operation.result {
                    Some(Ok(())) => format!("{} completed", operation.action.label()),
                    Some(Err(error)) => error.clone(),
                    None => "Operation ended".into(),
                }
            };
            let count =
                operation.output.lines.len() + usize::from(!operation.output.partial.is_empty());
            let widest = operation
                .output
                .lines
                .iter()
                .map(|line| line.len())
                .chain(std::iter::once(operation.output.partial.len()))
                .enumerate()
                .max_by_key(|(_, length)| *length)
                .map(|(index, _)| index);
            let project_name = name.to_owned();
            let output = uniform_list(
                "compose-log-scroll",
                count.max(1),
                cx.processor(move |this, range: std::ops::Range<usize>, _, _| {
                    let operation = &this.projects.operations[&project_name];
                    range
                        .map(|index| {
                            let text = if let Some(line) = operation.output.lines.get(index) {
                                line.render_on("", colors.text, colors.background)
                            } else if !operation.output.partial.is_empty() {
                                operation.output.partial.render_on(
                                    "",
                                    colors.text,
                                    colors.background,
                                )
                            } else {
                                StyledText::new(if operation.running {
                                    "Waiting for Compose output…"
                                } else {
                                    "Compose produced no output."
                                })
                            };
                            div()
                                .h(px(18.0))
                                .whitespace_nowrap()
                                .id(SharedString::from(format!("compose-log-line-{index}")))
                                .test_support()
                                .debug_selector(move || format!("compose-log-line-{index}"))
                                .child(text)
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .with_width_from_item(widest)
            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
            .track_scroll(&operation.output_scroll)
            .flex_1()
            .min_h_0()
            .font_family(super::ui::MONOSPACE_FONT)
            .text_size(px(12.0));
            body = body
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(operation.running, |view| view.child(Spinner::new().small()))
                        .child(
                            (if operation.result.as_ref().is_some_and(Result::is_err) {
                                Tag::danger()
                            } else if operation.running {
                                Tag::primary()
                            } else {
                                Tag::success()
                            })
                            .small()
                            .outline()
                            .child(status),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .child(output)
                        .child(
                            Scrollbar::new(&operation.output_scroll)
                                .id("compose-log-scrollbar")
                                .mode(ScrollbarMode::Always),
                        ),
                );
        } else if let Some(config) = project.and_then(|project| project.config.as_ref()) {
            let paths = |values: &[std::path::PathBuf]| {
                values
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" → ")
            };
            body = body.child(DescriptionList::new().columns(1).children([
                DescriptionItem::new("Directory").value(config.directory.display().to_string()),
                DescriptionItem::new("Compose files").value(paths(&config.files)),
                DescriptionItem::new("Environment files").value(if config.env_files.is_empty() { "default .env".into() } else { paths(&config.env_files) }),
                DescriptionItem::new("Profiles").value(if config.profiles.is_empty() { "default services".into() } else { config.profiles.join(", ") }),
            ])).child(Alert::info("compose-operation-info", "Start creates or starts services without building images. Stop and Restart operate on project services. Volumes are preserved."));
        } else {
            body = body.child(empty_state("Associate Compose files", "Review environment files and profiles before managing this project. Individual container controls remain available.")
                .child(Self::button("compose-associate-empty", "Associate files", false, self.connection.is_none(), cx)
                    .on_click(cx.listener(|this, _, window, cx| this.configure_project(window, cx)))));
        }
        pane.child(body)
    }
}
