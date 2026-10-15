use super::*;
use gpui_kit::{
    base::TestSupportExt,
    component::{dialog::Dialog, toolbar::Toolbar},
};
impl FerryDock {
    pub(in crate::app) fn open_quick_console(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.favorites.quick.sessions.is_empty() {
            self.open_quick_launcher(window, cx);
            return;
        }
        let focus = self
            .favorites
            .quick
            .return_focus
            .clone()
            .or_else(|| window.focused(cx));
        if self.favorites.quick.launcher.is_some() || self.favorites.quick.console_open {
            window.close_dialog(cx);
        }
        self.favorites.quick.return_focus = focus;
        self.favorites.quick.launcher = None;
        self.favorites.quick.console_open = true;
        self.favorites.quick.active = self
            .favorites
            .quick
            .active
            .or_else(|| self.favorites.quick.sessions.last().map(|s| s.generation));
        let owner = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            owner
                .update(cx, |this, cx| this.quick_console_dialog(dialog, cx))
                .unwrap_or_else(|_| Dialog::new(cx))
        });
        if let Some(s) = self
            .favorites
            .quick
            .sessions
            .iter()
            .find(|s| Some(s.generation) == self.favorites.quick.active)
        {
            s.view.update(cx, |v, cx| v.focus(window, cx));
        }
        cx.notify();
    }
    pub(in crate::app) fn quick_runs_label(&self, cx: &App) -> String {
        let active = self
            .favorites
            .quick
            .sessions
            .iter()
            .filter(|s| {
                s.pending.is_some()
                    || s.view.read(cx).favorite.as_ref().is_some_and(|f| {
                        matches!(f.status, FavoriteStatus::Starting | FavoriteStatus::Running)
                    })
            })
            .count();
        if active > 0 {
            format!("Quick runs · {active} running")
        } else {
            "Quick runs".into()
        }
    }
    fn quick_console_dialog(&self, dialog: Dialog, cx: &mut Context<Self>) -> Dialog {
        let colors = crate::app::ui::palette(cx);
        let mut body = div().flex().flex_col().h(px(510.0)).min_h_0().gap_2();
        let mut sessions = div().flex().flex_wrap().gap_2();
        for (index, s) in self.favorites.quick.sessions.iter().enumerate() {
            let generation = s.generation;
            sessions = sessions.child(
                Self::button(
                    SharedString::from(format!("quick-session-{generation}")),
                    format!("{} · {}", index + 1, s.name),
                    Some(generation) == self.favorites.quick.active,
                    false,
                    cx,
                )
                .max_w(px(260.0))
                .overflow_hidden()
                .tooltip(Self::quick_status(s, cx).0)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.favorites.quick.active = Some(generation);
                    if let Some(s) = this
                        .favorites
                        .quick
                        .sessions
                        .iter()
                        .find(|s| s.generation == generation)
                    {
                        s.view.update(cx, |v, cx| v.focus(window, cx));
                    }
                    cx.notify();
                })),
            );
        }
        body = body.child(sessions);
        if let Some(s) = self
            .favorites
            .quick
            .sessions
            .iter()
            .find(|s| Some(s.generation) == self.favorites.quick.active)
        {
            let (status, error) = Self::quick_status(s, cx);
            let generation = s.generation;
            let pending = s.pending.is_some();
            body = body
                .child(div().text_sm().text_ellipsis().child(format!(
                    "{} · {}",
                    s.target.label(),
                    s.name
                )))
                .child(
                    div()
                        .id("quick-run-status")
                        .test_support()
                        .text_color(rgb(if error { colors.error } else { colors.accent }))
                        .child(status),
                )
                .child(
                    div()
                        .id("quick-run-terminal")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .child(s.view.clone()),
                )
                .child(
                    Toolbar::new("quick-console-toolbar")
                        .content(
                            div()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(colors.muted))
                                .text_ellipsis()
                                .child(s.view.read(cx).status.clone()),
                        )
                        .child(
                            Self::button("quick-end-session", "End session", false, false, cx)
                                .tooltip("Disconnect this shell and release its terminal")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.backend
                                        .send(Command::CloseQuickTerminal { generation });
                                    this.favorites
                                        .quick
                                        .sessions
                                        .retain(|s| s.generation != generation);
                                    this.favorites.quick.active =
                                        this.favorites.quick.sessions.last().map(|s| s.generation);
                                    if this.favorites.quick.sessions.is_empty() {
                                        window.close_dialog(cx);
                                        this.close_quick_surface(window, cx);
                                    } else {
                                        if let Some(s) = this.favorites.quick.sessions.last() {
                                            s.view.update(cx, |v, cx| v.focus(window, cx));
                                        }
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .when(pending, |body| {
                    body.child(
                        div()
                            .text_sm()
                            .child("The command will run once the shell is ready."),
                    )
                });
        }
        dialog
            .title("Quick runs")
            .width(px(1000.0))
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .on_close(cx.listener(|this, _, window, cx| this.close_quick_surface(window, cx)))
            .child(body)
            .footer(
                div()
                    .flex()
                    .justify_between()
                    .w_full()
                    .child(
                        Self::button("quick-another", "Run another…", false, false, cx).on_click(
                            cx.listener(|this, _, window, cx| this.open_quick_launcher(window, cx)),
                        ),
                    )
                    .child(
                        Self::button("quick-hide", "Hide", false, false, cx)
                            .tooltip("Keep commands running and return to the workspace")
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.close_dialog(cx);
                                this.close_quick_surface(window, cx);
                            })),
                    ),
            )
    }
}
