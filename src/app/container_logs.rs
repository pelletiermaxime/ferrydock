//! Container log state and virtualized log rendering.
use super::FerryDock;
use super::ui::palette;
use crate::ansi::Buffer as StyledLogBuffer;
use crate::docker::Command;
use gpui::{prelude::*, *};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{status_bar::StatusBar, toolbar::Toolbar};

pub(super) struct ContainerLogs {
    pub(super) generation: u64,
    pub(super) buffer: StyledLogBuffer,
    pub(super) status: String,
    pub(super) follow: bool,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) reopen: bool,
}
impl Default for ContainerLogs {
    fn default() -> Self {
        Self {
            generation: 0,
            buffer: StyledLogBuffer::with_timestamps(),
            status: "Select a container to view its logs".into(),
            follow: true,
            scroll: UniformListScrollHandle::default(),
            reopen: true,
        }
    }
}

impl FerryDock {
    pub(super) fn logs_view(&self, cx: &mut Context<Self>) -> Div {
        let colors = palette(cx);
        let count = self.container_logs.buffer.lines.len()
            + usize::from(!self.container_logs.buffer.partial.is_empty());
        let widest = self
            .container_logs
            .buffer
            .lines
            .iter()
            .map(|line| line.len())
            .chain(std::iter::once(self.container_logs.buffer.partial.len()))
            .enumerate()
            .max_by_key(|(_, length)| *length)
            .map(|(index, _)| index);
        let lines = uniform_list(
            "log-scroll",
            count.max(1),
            cx.processor(move |this, range: std::ops::Range<usize>, _, _| {
                range
                    .map(|index| {
                        let text = if let Some(line) = this.container_logs.buffer.lines.get(index) {
                            line.render_on("", colors.text, colors.logs)
                        } else if this.container_logs.buffer.partial.is_empty() {
                            StyledText::new("Waiting for log output…")
                        } else {
                            this.container_logs.buffer.partial.render_on(
                                "",
                                colors.text,
                                colors.logs,
                            )
                        };
                        div()
                            .h(px(18.0))
                            .whitespace_nowrap()
                            .id(SharedString::from(format!("container-log-line-{index}")))
                            .test_support()
                            .debug_selector(move || format!("container-log-line-{index}"))
                            .child(text)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .with_width_from_item(widest)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .track_scroll(&self.container_logs.scroll)
        .flex_1()
        .min_h_0()
        .p_3()
        .font_family("DejaVu Sans Mono")
        .text_size(px(12.0));
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(colors.logs))
            .child(
                Toolbar::new("container-log-toolbar")
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(colors.border))
                    .child(
                        Self::button(
                            "follow",
                            if self.container_logs.follow {
                                "Following"
                            } else {
                                "Follow paused"
                            },
                            self.container_logs.follow,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.container_logs.follow = !this.container_logs.follow;
                            if this.container_logs.follow {
                                this.container_logs.scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Self::button("copy-logs", "Copy logs", false, false, cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.container_logs.buffer.text(),
                                ));
                                Self::notify("Retained logs copied", false, window, cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        Self::button(
                            "refresh-logs",
                            "Refresh logs",
                            false,
                            self.connection.is_none(),
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = &this.containers.selected
                                && this.connection.is_some()
                            {
                                this.container_logs.generation += 1;
                                this.container_logs.buffer = StyledLogBuffer::with_timestamps();
                                this.backend.send(Command::Logs {
                                    id: id.clone(),
                                    generation: this.container_logs.generation,
                                });
                                this.backend.send(Command::Inspect {
                                    id: id.clone(),
                                    generation: this.container_logs.generation,
                                });
                                this.container_logs.status = "Connecting to logs…".into();
                                cx.notify();
                            }
                        })),
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
                    .id("container-log-viewport")
                    .test_support()
                    .debug_selector(|| "container-log-viewport".to_owned())
                    .child(lines)
                    .child(
                        Scrollbar::new(&self.container_logs.scroll)
                            .id("container-log-scrollbar")
                            .mode(ScrollbarMode::Always),
                    ),
            )
            .child(StatusBar::new().left(self.container_logs.status.clone()))
    }
}
