//! Shared visual theme, controls, notifications, and dock sizing.
use super::FerryDock;
use gpui::{prelude::*, *};
use gpui_kit::component::Disableable;
use gpui_kit::component::Selectable;
use gpui_kit::component::Sizable;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::notification::NotificationDelivery;

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: u32,
    pub panel: u32,
    pub text: u32,
    pub muted: u32,
    pub border: u32,
    pub accent: u32,
    pub selected: u32,
    pub error: u32,
    pub logs: u32,
}

pub(crate) fn palette(cx: &App) -> Palette {
    use gpui_kit::component::ActiveTheme;
    if cx.global::<super::appearance::Appearance>().theme == super::appearance::AppTheme::FerryDock
    {
        return Palette {
            background: 0x171d21,
            panel: 0x1e272c,
            text: 0xe0ece7,
            muted: 0x9eb2a8,
            border: 0x34433c,
            accent: 0x74d7b1,
            selected: 0x234136,
            error: 0xffa4a4,
            logs: 0x101819,
        };
    }
    let theme = cx.theme();
    let color = |value: Hsla| u32::from(value.to_rgb()) >> 8;
    Palette {
        background: color(theme.background),
        panel: color(theme.sidebar),
        text: color(theme.foreground),
        muted: color(theme.muted_foreground),
        border: color(theme.border),
        accent: color(theme.primary),
        selected: color(theme.selection),
        error: color(theme.danger),
        logs: color(theme.background),
    }
}

pub fn init_ui(cx: &mut App) {
    gpui_kit::init(cx);
    super::appearance::init(cx);
}

pub(super) fn empty_state(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
) -> gpui_kit::component::empty::Empty {
    use gpui_kit::component::empty::*;
    Empty::new().header(
        EmptyHeader::new()
            .title(EmptyTitle::new().child(title.into()))
            .description(EmptyDescription::new().child(description.into())),
    )
}

pub(super) struct FeedbackNotification;

impl FerryDock {
    pub(super) fn notify(
        message: impl Into<SharedString>,
        error: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let message = message.into();
        let notification = if error {
            Notification::error(message).autohide(false)
        } else {
            Notification::success(message)
        };
        window.push_notification(
            notification
                .id::<FeedbackNotification>()
                .delivery(NotificationDelivery::InApp),
            cx,
        );
    }

    pub(super) fn expand_dock(&mut self, height: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |dock, cx| {
            if dock.sizes().get(1).copied().unwrap_or(px(340.0)) < px(height) {
                dock.resize_panel(1, px(height), window, cx);
            }
        });
    }

    pub(super) fn button(
        id: impl Into<SharedString>,
        text: impl Into<SharedString>,
        active: bool,
        disabled: bool,
        cx: &App,
    ) -> Button {
        let colors = palette(cx);
        let id = id.into();
        let tooltip = match id.as_ref() {
            "refresh" => Some("Refresh containers (Ctrl+R)"),
            "copy-logs" | "project-log-copy" => Some("Copy all retained logs"),
            "compose-configure" => Some("Review Compose files and project settings"),
            "shell-auto" => Some("Try Bash, then fall back to sh"),
            _ => None,
        };
        Button::new(id)
            .small()
            .outline()
            .label(text)
            .selected(active)
            .disabled(disabled)
            .bg(rgb(if active {
                colors.selected
            } else {
                colors.panel
            }))
            .text_color(rgb(if active { colors.accent } else { colors.text }))
            .when(disabled, |button| button.opacity(0.45))
            .when_some(tooltip, |button, tooltip| button.tooltip(tooltip))
    }
}
