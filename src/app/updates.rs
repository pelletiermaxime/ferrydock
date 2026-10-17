//! AppImage update state, Settings controls and deliberate restart review.
use super::FerryDock;
use crate::updater::{Command, Event, Worker};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, WindowExt as _,
    button::Button,
    checkbox::Checkbox,
    dialog::Dialog,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage},
};

pub(super) struct Updates {
    worker: Option<Worker>,
    automatic: bool,
    loaded: bool,
    unavailable: Option<String>,
    status: String,
    busy: bool,
    pub(super) applying: bool,
    version: Option<String>,
    error: bool,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            worker: None,
            automatic: true,
            loaded: false,
            unavailable: None,
            status: "Loading update settings…".into(),
            busy: false,
            applying: false,
            version: None,
            error: false,
        }
    }
}
impl Updates {
    pub(super) fn start() -> Self {
        Self {
            worker: Some(Worker::start()),
            ..Self::default()
        }
    }
    pub(super) fn ready(&self) -> bool {
        self.version.is_some()
    }

    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn fixture(worker: Worker) -> Self {
        Self {
            worker: Some(worker),
            ..Self::default()
        }
    }

    fn send(&mut self, command: Command) {
        if self.busy {
            return;
        }
        if let Some(worker) = &self.worker {
            match worker.commands.try_send(command) {
                Ok(()) => {
                    self.busy = true;
                    self.error = false;
                }
                Err(_) => {
                    self.status =
                        "The update worker is unavailable; restart FerryDock to try again.".into();
                    self.error = true;
                }
            }
        }
    }
}

impl FerryDock {
    pub(super) fn poll_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for _ in 0..8 {
            let Some(event) = self
                .updates
                .worker
                .as_ref()
                .and_then(|worker| worker.events.try_recv().ok())
            else {
                break;
            };
            changed = true;
            match event {
                Event::Loaded {
                    automatic,
                    unavailable,
                } => {
                    self.updates.loaded = true;
                    self.updates.automatic = automatic;
                    self.updates.unavailable = unavailable;
                    self.updates.status =
                        "Automatic checks run after startup and every six hours.".into();
                }
                Event::Automatic(enabled) => {
                    self.updates.automatic = enabled;
                    self.updates.busy = false;
                }
                Event::Checking => {
                    self.updates.busy = true;
                    self.updates.error = false;
                    self.updates.status = "Checking GitHub Releases…".into();
                }
                Event::Downloading {
                    version,
                    bytes,
                    total,
                } => {
                    self.updates.busy = true;
                    self.updates.status = format!(
                        "Downloading {version} · {:.1} / {:.1} MiB",
                        bytes as f64 / 1_048_576.0,
                        total as f64 / 1_048_576.0
                    );
                }
                Event::Ready(version) => {
                    self.updates.status =
                        format!("FerryDock {version} is ready. Download verified with SHA-256.");
                    self.updates.version = Some(version);
                    self.updates.busy = false;
                    self.updates.error = false;
                }
                Event::Current => {
                    self.updates.busy = false;
                    self.updates.status = "You’re using the latest stable release.".into();
                    self.updates.error = false;
                }
                Event::Error(error) => {
                    if self.updates.applying {
                        self.updates.applying = false;
                        window.close_dialog(cx);
                        Self::notify(error.clone(), true, window, cx);
                    }
                    self.updates.busy = false;
                    self.updates.version = None;
                    self.updates.status = error;
                    self.updates.error = true;
                }
                Event::Relaunched => cx.quit(),
            }
        }
        changed
    }

    fn update_restart_blocked(&self) -> bool {
        !self.containers.busy.is_empty()
            || self.projects.operations.values().any(|op| op.running)
            || self.volumes.mutating()
            || self.images.pruning.is_some()
            || !self.images.busy.is_empty()
            || self.favorite_save_pending()
    }

    pub(super) fn update_settings(&self, cx: &mut Context<Self>) -> SettingPage {
        let owner = cx.entity().downgrade();
        let automatic_owner = owner.clone();
        let status_owner = owner.clone();
        SettingPage::new("Updates").group(SettingGroup::new().title("AppImage updates")
            .description(format!("Installed version: {}. Stable updates come from FerryDock’s GitHub Releases.", env!("CARGO_PKG_VERSION")))
            .item(SettingItem::new("Automatic updates", SettingField::render(move |_, _, cx| {
                automatic_owner.update(cx, |this, cx| {
                    Checkbox::new("update-automatic").debug_selector(|| "update-automatic".into())
                        .label("Automatically check and download")
                        .checked(this.updates.automatic)
                        .disabled(!this.updates.loaded || this.updates.busy || this.updates.unavailable.is_some())
                        .on_click(cx.listener(|this, enabled, _, cx| { this.updates.send(Command::Automatic(*enabled)); cx.notify(); }))
                }).unwrap().into_any_element()
            })).layout(Axis::Vertical).description("Checks run after startup and every six hours. You choose when to restart and apply a downloaded update."))
            .item(SettingItem::new("Update status", SettingField::render(move |_, _, cx| {
                status_owner.update(cx, |this, cx| {
                    let unavailable = this.updates.unavailable.clone();
                    div().flex().flex_col().gap_3()
                        .child(div().max_w(px(460.0)).text_sm()
                            .text_color(if this.updates.error { cx.theme().danger } else { cx.theme().muted_foreground })
                            .child(unavailable.unwrap_or_else(|| this.updates.status.clone())))
                        .child(div().flex().flex_wrap().gap_2()
                            .child(Button::new("update-check").debug_selector(|| "update-check".into()).small().outline().label("Check for updates")
                                .loading(this.updates.busy).disabled(!this.updates.loaded || this.updates.busy || this.updates.ready() || this.updates.unavailable.is_some())
                                .on_click(cx.listener(|this, _, _, cx| { this.updates.send(Command::Check); cx.notify(); })))
                            .when(this.updates.ready(), |row| row.child(this.update_restart_button("update-restart-settings", cx)))
                            .child(Button::new("update-releases").small().label("Release notes")
                                .on_click(|_, _, cx| cx.open_url("https://github.com/pelletiermaxime/ferrydock/releases"))))
                        .into_any_element()
                }).unwrap()
            })).layout(Axis::Vertical))
            .item(SettingItem::new("Download verification", SettingField::render(|_, _, _| {
                div().text_sm().max_w(px(460.0)).child("Downloads are checked against the release’s SHA256SUMS. Publisher signatures are not used in this version.")
            })).layout(Axis::Vertical)))
    }

    fn update_restart_button(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        Self::button(
            id,
            "Restart to update",
            false,
            self.updates.busy || self.update_restart_blocked(),
            cx,
        )
        .debug_selector(move || id.into())
        .tooltip(if self.update_restart_blocked() {
            "Wait for Docker and Compose operations to finish"
        } else {
            "Review and restart FerryDock to apply the downloaded update"
        })
        .on_click(cx.listener(|this, _, window, cx| this.review_update_restart(window, cx)))
    }

    pub(super) fn update_banner(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().child(format!(
                "FerryDock {} is ready to install",
                self.updates.version.as_deref().unwrap_or_default()
            )))
            .child(self.update_restart_button("update-restart", cx))
    }

    fn review_update_restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.updates.ready() || self.updates.busy || self.update_restart_blocked() {
            return;
        }
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            owner.update(cx, |this, cx| {
                let live_shell = this.terminal.view.as_ref().is_some_and(|v| v.read(cx).ready) || this.live_quick_runs(cx);
                dialog.title(if this.updates.applying { "Applying update…" } else { "Restart FerryDock to update?" }).width(px(520.0))
                    .overlay_closable(false).keyboard(!this.updates.applying).close_button(!this.updates.applying)
                    .child(format!("Install version {} and reopen FerryDock. Running containers stay running.", this.updates.version.as_deref().unwrap_or_default()))
                    .when(live_shell, |dialog| dialog.child(div().text_color(cx.theme().warning).child("Open terminal and quick-run sessions will close. Finish any commands you want to keep before restarting.")))
                    .footer(div().flex().justify_end().gap_2()
                        .child(Self::button("update-restart-cancel", "Later", false, this.updates.applying, cx).debug_selector(|| "update-restart-cancel".into()).on_click(|_, window, cx| window.close_dialog(cx)))
                        .child(Self::button("update-restart-confirm", "Restart and apply", false, this.update_restart_blocked() || this.updates.busy, cx)
                            .debug_selector(|| "update-restart-confirm".into())
                            .on_click(cx.listener(|this, _, _window, cx| {
                                if this.updates.ready() && !this.update_restart_blocked() && !this.updates.busy {
                                    this.updates.status = "Applying update and restarting…".into();
                                    this.updates.send(Command::Apply);
                                    this.updates.applying = this.updates.busy;
                                    cx.notify();
                                }
                            }))))
            }).unwrap_or_else(|_| Dialog::new(cx).title("Update unavailable"))
        });
    }
}
