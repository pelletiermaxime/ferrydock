use std::sync::mpsc;

use alacritty_terminal::{
    event::{Event as TerminalEvent, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as TerminalPoint, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term, TermMode, cell::Flags},
    vte::ansi::{Color, NamedColor, Processor},
};
use gpui::{prelude::*, *};

use crate::{ansi::PALETTE, docker::Command};

const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 19.0;
const PADDING: f32 = 10.0;
const FOREGROUND: u32 = 0xe0ece7;
const BACKGROUND: u32 = 0x101819;

struct Size {
    columns: usize,
    rows: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

struct Listener(mpsc::Sender<String>);
impl EventListener for Listener {
    fn send_event(&self, event: TerminalEvent) {
        if let TerminalEvent::PtyWrite(text) = event {
            let _ = self.0.send(text);
        }
    }
}

pub struct TerminalCore {
    term: Term<Listener>,
    parser: Processor,
    replies: mpsc::Receiver<String>,
    pub columns: u16,
    pub rows: u16,
}

impl TerminalCore {
    pub fn new(columns: u16, rows: u16) -> Self {
        let (sender, replies) = mpsc::channel();
        let config = Config {
            scrolling_history: 5000,
            ..Default::default()
        };
        Self {
            term: Term::new(
                config,
                &Size {
                    columns: columns.into(),
                    rows: rows.into(),
                },
                Listener(sender),
            ),
            parser: Processor::new(),
            replies,
            columns,
            rows,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, columns: u16, rows: u16) -> bool {
        if (columns, rows) == (self.columns, self.rows) {
            return false;
        }
        self.term.resize(Size {
            columns: columns.into(),
            rows: rows.into(),
        });
        self.columns = columns;
        self.rows = rows;
        true
    }

    pub fn replies(&self) -> Vec<String> {
        self.replies.try_iter().collect()
    }

    pub fn visible_text(&self) -> String {
        let mut lines = vec![String::new(); self.rows as usize];
        let content = self.term.renderable_content();
        let offset = content.display_offset as i32;
        for indexed in content.display_iter {
            let line = indexed.point.line.0 + offset;
            if line >= 0
                && line < self.rows as i32
                && !indexed.cell.flags.contains(Flags::WIDE_CHAR_SPACER)
            {
                lines[line as usize].push(indexed.cell.c);
            }
        }
        lines
            .into_iter()
            .map(|line| line.trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn paste(&self, text: &str) -> Vec<u8> {
        // Pasting ESC into bracketed mode must not let clipboard content close
        // the bracket early. Normalise platform line endings for a Linux PTY.
        let text = text
            .replace('\u{1b}', "")
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if self.term.mode().contains(TermMode::BRACKETED_PASTE) {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.replace('\n', "\r").into_bytes()
        }
    }
}

/// Xterm sequences, including application cursor mode and modified arrows.
pub fn key_bytes(
    key: &str,
    text: Option<&str>,
    control: bool,
    alt: bool,
    shift: bool,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let arrow = match key {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(arrow) = arrow {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{arrow}")
            } else if application_cursor {
                format!("\x1bO{arrow}")
            } else {
                format!("\x1b[{arrow}")
            }
            .into_bytes(),
        );
    }
    if control {
        let code = match key {
            "space" | "@" | "2" => Some(0),
            "[" | "3" => Some(27),
            "\\" | "4" => Some(28),
            "]" | "5" => Some(29),
            "^" | "6" => Some(30),
            "_" | "7" | "/" => Some(31),
            "8" | "backspace" => Some(127),
            _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
                Some(key.as_bytes()[0].to_ascii_lowercase() - b'a' + 1)
            }
            _ => None,
        };
        if let Some(code) = code {
            return Some(if alt { vec![27, code] } else { vec![code] });
        }
    }
    let special = match key {
        "enter" => Some("\r"),
        "escape" => Some("\x1b"),
        "backspace" => Some("\x7f"),
        "tab" if shift => Some("\x1b[Z"),
        "tab" => Some("\t"),
        "insert" => Some("\x1b[2~"),
        "delete" => Some("\x1b[3~"),
        "pageup" => Some("\x1b[5~"),
        "pagedown" => Some("\x1b[6~"),
        "f1" => Some("\x1bOP"),
        "f2" => Some("\x1bOQ"),
        "f3" => Some("\x1bOR"),
        "f4" => Some("\x1bOS"),
        "f5" => Some("\x1b[15~"),
        "f6" => Some("\x1b[17~"),
        "f7" => Some("\x1b[18~"),
        "f8" => Some("\x1b[19~"),
        "f9" => Some("\x1b[20~"),
        "f10" => Some("\x1b[21~"),
        "f11" => Some("\x1b[23~"),
        "f12" => Some("\x1b[24~"),
        _ => None,
    };
    let mut bytes = special.or(text)?.as_bytes().to_vec();
    if alt {
        bytes.insert(0, 27);
    }
    Some(bytes)
}

fn ansi_color(color: Color, colors: &alacritty_terminal::term::color::Colors) -> u32 {
    match color {
        Color::Spec(color) => {
            (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
        }
        Color::Named(named) => {
            if let Some(color) = colors[named as usize] {
                return (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b);
            }
            match named {
                NamedColor::Background => BACKGROUND,
                NamedColor::Foreground | NamedColor::Cursor => FOREGROUND,
                _ => PALETTE.get(named as usize).copied().unwrap_or(FOREGROUND),
            }
        }
        Color::Indexed(index) if index < 16 => PALETTE[index as usize],
        Color::Indexed(index) if index >= 232 => {
            let value = 8 + u32::from(index - 232) * 10;
            (value << 16) | (value << 8) | value
        }
        Color::Indexed(index) => {
            let index = index - 16;
            let scale = |value: u8| {
                if value == 0 {
                    0
                } else {
                    55 + u32::from(value) * 40
                }
            };
            (scale(index / 36) << 16) | (scale(index / 6 % 6) << 8) | scale(index % 6)
        }
    }
}

pub struct TerminalView {
    pub core: TerminalCore,
    pub generation: u64,
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    focus: FocusHandle,
    bounds: Bounds<Pixels>,
    cell_width: f32,
    dragging: bool,
    pub ready: bool,
    pub status: String,
}

impl TerminalView {
    pub fn new(
        generation: u64,
        commands: tokio::sync::mpsc::UnboundedSender<Command>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            core: TerminalCore::new(80, 24),
            generation,
            commands,
            focus: cx.focus_handle(),
            bounds: Bounds::default(),
            cell_width: 8.0,
            dragging: false,
            ready: false,
            status: "Opening shell…".into(),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub fn feed(&mut self, data: &[u8], cx: &mut Context<Self>) {
        self.core.feed(data);
        for reply in self.core.replies() {
            self.send(reply.into_bytes());
        }
        cx.notify();
    }

    fn send(&self, data: Vec<u8>) {
        if self.ready {
            let _ = self.commands.send(Command::Input {
                generation: self.generation,
                data,
            });
        }
    }

    pub fn copy(&self, cx: &mut App) {
        if let Some(text) = self.core.term.selection_to_string() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.core.term.scroll_display(Scroll::Bottom);
            self.send(self.core.paste(&text));
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.control && key.modifiers.shift && key.key == "c" {
            self.copy(cx);
            cx.stop_propagation();
            return;
        }
        if (key.modifiers.control && key.modifiers.shift && key.key == "v")
            || (key.modifiers.shift && key.key == "insert")
        {
            self.paste(cx);
            cx.stop_propagation();
            return;
        }
        if key.modifiers.shift && (key.key == "pageup" || key.key == "pagedown") {
            self.core.term.scroll_display(if key.key == "pageup" {
                Scroll::PageUp
            } else {
                Scroll::PageDown
            });
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if let Some(bytes) = key_bytes(
            &key.key,
            key.key_char.as_deref(),
            key.modifiers.control,
            key.modifiers.alt,
            key.modifiers.shift,
            self.core.term.mode().contains(TermMode::APP_CURSOR),
        ) {
            self.core.term.selection = None;
            self.core.term.scroll_display(Scroll::Bottom);
            self.send(bytes);
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn point_at(&self, position: Point<Pixels>) -> TerminalPoint {
        let column = ((f32::from(position.x - self.bounds.origin.x) - PADDING) / self.cell_width)
            .floor()
            .max(0.0) as usize;
        let line = ((f32::from(position.y - self.bounds.origin.y) - PADDING) / LINE_HEIGHT)
            .floor()
            .max(0.0) as i32;
        TerminalPoint::new(
            Line(
                line.min(i32::from(self.core.rows) - 1)
                    - self.core.term.grid().display_offset() as i32,
            ),
            Column(column.min(usize::from(self.core.columns) - 1)),
        )
    }
}

struct Prepared {
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    backgrounds: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let view = entity.clone();
        div()
            .id("terminal")
            .size_full()
            .bg(rgb(BACKGROUND))
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus(window, cx);
                    this.dragging = true;
                    this.core.term.selection = Some(Selection::new(
                        SelectionType::Simple,
                        this.point_at(event.position),
                        Side::Left,
                    ));
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.dragging {
                    let point = this.point_at(event.position);
                    if let Some(selection) = &mut this.core.term.selection {
                        selection.update(point, Side::Right);
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.dragging = false;
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
                this.core
                    .term
                    .scroll_display(Scroll::Delta(if delta.y > px(0.0) { 3 } else { -3 }));
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        view.update(cx, |this, cx| {
                            this.bounds = bounds;
                            let font = font("DejaVu Sans Mono");
                            let sample = TextRun {
                                len: 1,
                                font: font.clone(),
                                color: rgb_to_hsla(rgb(FOREGROUND)),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                                letter_spacing: None,
                            };
                            this.cell_width = f32::from(
                                window
                                    .text_system()
                                    .shape_line("M".into(), px(FONT_SIZE), &[sample])
                                    .width,
                            );
                            let columns = ((f32::from(bounds.size.width) - 2.0 * PADDING)
                                / this.cell_width)
                                .floor()
                                .clamp(2.0, 500.0) as u16;
                            let rows = ((f32::from(bounds.size.height) - 2.0 * PADDING)
                                / LINE_HEIGHT)
                                .floor()
                                .clamp(1.0, 200.0) as u16;
                            if this.core.resize(columns, rows) {
                                let _ = this.commands.send(Command::Resize {
                                    generation: this.generation,
                                    columns,
                                    rows,
                                });
                                cx.notify();
                            }
                            let content = this.core.term.renderable_content();
                            let offset = content.display_offset as i32;
                            let mut text = vec![String::new(); usize::from(rows)];
                            let mut runs = vec![Vec::new(); usize::from(rows)];
                            let mut backgrounds = Vec::new();
                            for indexed in content.display_iter {
                                let row = indexed.point.line.0 + offset;
                                if row < 0 || row >= i32::from(rows) {
                                    continue;
                                }
                                let cell = indexed.cell;
                                let foreground = ansi_color(
                                    if cell.flags.contains(Flags::INVERSE) {
                                        cell.bg
                                    } else {
                                        cell.fg
                                    },
                                    content.colors,
                                );
                                let mut background = ansi_color(
                                    if cell.flags.contains(Flags::INVERSE) {
                                        cell.fg
                                    } else {
                                        cell.bg
                                    },
                                    content.colors,
                                );
                                if content
                                    .selection
                                    .as_ref()
                                    .is_some_and(|range| range.contains(indexed.point))
                                {
                                    background = 0x36574a;
                                }
                                if background != BACKGROUND {
                                    backgrounds.push(fill(
                                        Bounds::new(
                                            bounds.origin
                                                + point(
                                                    px(PADDING
                                                        + indexed.point.column.0 as f32
                                                            * this.cell_width),
                                                    px(PADDING + row as f32 * LINE_HEIGHT),
                                                ),
                                            size(px(this.cell_width), px(LINE_HEIGHT)),
                                        ),
                                        rgb(background),
                                    ));
                                }
                                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                                    continue;
                                }
                                let mut value = cell.c.to_string();
                                if let Some(extra) = cell.zerowidth() {
                                    value.extend(extra.iter());
                                }
                                let mut cell_font = font.clone();
                                if cell.flags.contains(Flags::BOLD) {
                                    cell_font.weight = FontWeight::BOLD;
                                }
                                runs[row as usize].push(TextRun {
                                    len: value.len(),
                                    font: cell_font,
                                    color: rgb_to_hsla(rgb(foreground)),
                                    background_color: None,
                                    underline: cell.flags.contains(Flags::UNDERLINE).then_some(
                                        UnderlineStyle {
                                            color: Some(rgb_to_hsla(rgb(foreground))),
                                            thickness: px(1.0),
                                            wavy: false,
                                        },
                                    ),
                                    strikethrough: None,
                                    letter_spacing: None,
                                });
                                text[row as usize].push_str(&value);
                            }
                            let lines = text
                                .into_iter()
                                .zip(runs)
                                .enumerate()
                                .map(|(row, (text, runs))| {
                                    (
                                        bounds.origin
                                            + point(
                                                px(PADDING),
                                                px(PADDING + row as f32 * LINE_HEIGHT),
                                            ),
                                        window.text_system().shape_line(
                                            text.into(),
                                            px(FONT_SIZE),
                                            &runs,
                                        ),
                                    )
                                })
                                .collect();
                            let cursor_row = content.cursor.point.line.0 + offset;
                            let cursor = (this.focus.is_focused(window)
                                && content.mode.contains(TermMode::SHOW_CURSOR)
                                && cursor_row >= 0
                                && cursor_row < i32::from(rows))
                            .then(|| {
                                fill(
                                    Bounds::new(
                                        bounds.origin
                                            + point(
                                                px(PADDING
                                                    + content.cursor.point.column.0 as f32
                                                        * this.cell_width),
                                                px(PADDING + cursor_row as f32 * LINE_HEIGHT),
                                            ),
                                        size(px(2.0), px(LINE_HEIGHT)),
                                    ),
                                    rgb(0x74d7b1),
                                )
                            });
                            Prepared {
                                lines,
                                backgrounds,
                                cursor,
                            }
                        })
                    },
                    move |_, prepared, window, cx| {
                        for background in prepared.backgrounds {
                            window.paint_quad(background);
                        }
                        for (origin, line) in prepared.lines {
                            let _ = line.paint(
                                origin,
                                px(LINE_HEIGHT),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        if let Some(cursor) = prepared.cursor {
                            window.paint_quad(cursor);
                        }
                        let _ = &entity;
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{TerminalCore, key_bytes};

    #[test]
    fn parses_cursor_motion_alternate_screen_and_split_utf8() {
        let mut terminal = TerminalCore::new(20, 4);
        terminal.feed(b"abc\rZ\x1b[31m!\x1b[0m caf\xc3");
        terminal.feed(b"\xa9");
        assert!(terminal.visible_text().starts_with("Z! café"));
        terminal.feed(b"\x1b[?1049h\x1b[2J\x1b[Halternate");
        assert!(terminal.visible_text().contains("alternate"));
        terminal.feed(b"\x1b[?1049l");
        assert!(terminal.visible_text().starts_with("Z! café"));
        assert!(terminal.resize(40, 8));
        assert!(terminal.visible_text().contains("café"));
    }

    #[test]
    fn terminal_replies_and_bracketed_paste_are_protocol_correct() {
        let mut terminal = TerminalCore::new(80, 24);
        terminal.feed(b"\x1b[6n\x1b[?2004h");
        assert_eq!(terminal.replies(), vec!["\x1b[1;1R"]);
        assert_eq!(
            terminal.paste("hello\r\nworld\x1b[201~"),
            b"\x1b[200~hello\nworld[201~\x1b[201~"
        );
        assert_eq!(
            key_bytes("c", None, true, false, false, false),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes("up", None, false, false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes("left", None, true, false, false, false),
            Some(b"\x1b[1;5D".to_vec())
        );
    }
}
