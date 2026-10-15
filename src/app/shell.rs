//! Top-level window layout; feature panels render their own contents.
use super::FerryDock;
use super::ui::palette;
use gpui::{prelude::*, *};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    IconName, Sizable,
    alert::Alert,
    checkbox::Checkbox,
    input::Input,
    kbd::Kbd,
    resizable::{resizable_panel, v_resizable},
    sidebar::{Sidebar, SidebarFooter, SidebarGroup, SidebarMenuItem},
    spinner::Spinner,
    status_bar::StatusBar,
    tag::Tag,
    toolbar::Toolbar,
};

impl Render for FerryDock {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = palette(cx);
        let running = self
            .containers
            .items
            .iter()
            .filter(|container| container.running())
            .count();
        let connected = self.connection.is_some();
        let connection = self
            .connection
            .as_ref()
            .map(|connection| connection.name.clone())
            .unwrap_or_else(|| "Connecting to Docker…".into());
        let endpoint = self
            .connection
            .as_ref()
            .map(|connection| connection.endpoint.clone())
            .unwrap_or_default();
        let shortcuts = div().flex().flex_col().gap_2().children(
            [
                ("ctrl-k", "Run favorite"),
                ("ctrl-f", "Search"),
                ("ctrl-r", "Refresh"),
                ("ctrl-1", if self.images_open { "Overview" } else { "Logs" }),
                (
                    "ctrl-2",
                    if self.images_open {
                        "History"
                    } else {
                        "Terminal"
                    },
                ),
                ("ctrl-3", "Inspect"),
                (
                    "ctrl-4",
                    if self.images_open {
                        "Analysis"
                    } else {
                        "Project logs"
                    },
                ),
            ]
            .into_iter()
            .filter(|(key, _)| {
                !self.networks_open || matches!(*key, "ctrl-k" | "ctrl-f" | "ctrl-r")
            })
            .map(|(key, label)| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Kbd::new(Keystroke::parse(key).unwrap()))
                    .child(label)
            }),
        );
        let sidebar = Sidebar::new("workspace-sidebar")
            .w(px(165.0))
            .bg(rgb(colors.panel))
            .child(
                SidebarGroup::new("Workspace").children([
                    SidebarMenuItem::new("Containers")
                        .icon(IconName::LayoutDashboard)
                        .active(
                            !self.settings_open
                                && !self.images_open
                                && !self.volumes_open
                                && !self.networks_open,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_images(false, window, cx);
                        })),
                    SidebarMenuItem::new("Images")
                        .icon(gpui_kit::assets::IconName::HardDrive)
                        .active(!self.settings_open && self.images_open)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_images(true, window, cx);
                        })),
                    SidebarMenuItem::new("Volumes")
                        .icon(gpui_kit::assets::IconName::Database)
                        .active(!self.settings_open && self.volumes_open)
                        .on_click(cx.listener(|this, _, window, cx| this.show_volumes(window, cx))),
                    SidebarMenuItem::new("Networks")
                        .icon(IconName::Network)
                        .active(!self.settings_open && self.networks_open)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.show_networks(window, cx)),
                        ),
                    SidebarMenuItem::new("Settings")
                        .icon(IconName::Settings)
                        .active(self.settings_open)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_settings(true, window, cx);
                        })),
                ]),
            )
            .footer(SidebarFooter::new().child(shortcuts));
        let connection_tag = if connected {
            Tag::success()
        } else {
            Tag::secondary()
        };
        let header = Toolbar::new("app-toolbar")
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(rgb(colors.border))
            .content(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(19.0))
                    .child("FerryDock"),
            )
            .content(div().flex_1())
            .content(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(!connected, |view| view.child(Spinner::new().small()))
                    .child(connection_tag.small().outline().child(connection)),
            )
            .child(
                Self::button(
                    "quick-launcher",
                    "Run favorite…",
                    false,
                    self.connection.is_none(),
                    cx,
                )
                .tooltip("Run a saved command (Ctrl+K)")
                .on_click(cx.listener(|this, _, window, cx| this.open_quick_launcher(window, cx))),
            )
            .child(
                Self::button("quick-runs", self.quick_runs_label(cx), false, false, cx)
                    .tooltip("Open quick-run terminals and their results")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_quick_console(window, cx)),
                    ),
            )
            .child(
                Self::button("refresh", "Refresh", false, false, cx)
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh Docker resources (Ctrl+R)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh();
                        cx.notify();
                    })),
            )
            .child(
                Self::button("settings", "Settings", self.settings_open, false, cx)
                    .icon(IconName::Settings)
                    .tooltip("Appearance settings (Ctrl+,)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_settings(!this.settings_open, window, cx);
                    })),
            );
        let main = if self.settings_open {
            self.appearance_settings(cx).into_any_element()
        } else if self.networks_open {
            self.networks_workspace(cx).into_any_element()
        } else if self.volumes_open {
            self.volumes_workspace(cx).into_any_element()
        } else if self.images_open {
            self.images_workspace(cx).into_any_element()
        } else {
            let search = div()
                .id("search")
                .test_support()
                .w(px(260.0))
                .capture_key_down(cx.listener(Self::search_key))
                .child(Input::new(&self.containers.search_input).small());
            let mut main = div().flex_1().min_w_0().h_full().flex().flex_col().child(
                Toolbar::new("container-toolbar")
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .content(div().font_weight(FontWeight::MEDIUM).child("Containers"))
                    .content(div().flex_1())
                    .content(search)
                    .child(
                        Checkbox::new("all")
                            .small()
                            .label("Running only")
                            .checked(self.containers.running_only)
                            .on_change(cx.listener(|this, value: &bool, _, cx| {
                                this.containers.running_only = *value;
                                cx.notify();
                            })),
                    )
                    .when(!self.containers.running_only, |toolbar| {
                        toolbar.content(Tag::secondary().small().outline().child("All containers"))
                    }),
            );
            if let Some(error) = &self.connection_error {
                main = main.child(
                    Alert::error(
                        "docker-connection-error",
                        format!("{error} · retrying every 5 seconds"),
                    )
                    .title("Docker connection unavailable"),
                );
            }
            main.child(
                v_resizable("workspace-split")
                    .with_state(&self.dock)
                    .child(
                        resizable_panel().size_range(px(80.0)..Pixels::MAX).child(
                            div()
                                .id("containers-pane")
                                .test_support()
                                .debug_selector(|| "containers-pane".to_string())
                                .size_full()
                                .flex()
                                .flex_col()
                                .child(self.resources(cx)),
                        ),
                    )
                    .child(
                        resizable_panel()
                            .size(px(430.0))
                            .size_range(
                                px(if self.projects.selected.is_some() {
                                    200.0
                                } else {
                                    400.0
                                })..Pixels::MAX,
                            )
                            .child(
                                self.detail(cx)
                                    .id("detail-pane")
                                    .test_support()
                                    .debug_selector(|| "detail-pane".to_string()),
                            ),
                    ),
            )
            .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors.background))
            .text_color(rgb(colors.text))
            .font_family("Inter")
            .text_size(px(14.0))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::quick_run_key))
            .on_key_down(cx.listener(Self::root_key))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(sidebar)
                    .child(div().flex_1().min_w_0().child(main)),
            )
            .child(StatusBar::new().left(endpoint).right(format!(
                "{running} running / {} total",
                self.containers.items.len()
            )))
    }
}
