//! Incremental ANSI processing for log records, without creating terminal widgets.
use std::{collections::VecDeque, ops::Range};

use alacritty_terminal::vte::ansi::{Attr, Color, Handler, LineClearMode, NamedColor, Processor};
use gpui::{
    FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, StyledText, UnderlineStyle, px, rgb,
};

const MAX_BYTES: usize = 65_536;
const MAX_SPANS: usize = 512;
const FOREGROUND: u32 = 0xe0ece7;
const BACKGROUND: u32 = 0x101819;
pub(crate) const PALETTE: [u32; 16] = [
    0x162025, 0xe87979, 0x74d7b1, 0xe5c07b, 0x7eafe8, 0xbe9ae8, 0x73c9d0, 0xcbd8d2, 0x60746a,
    0xff9b9b, 0x9aebc8, 0xffdf9e, 0xa1c8ff, 0xd8b6ff, 0x9ce9ee, 0xf0f6f2,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub dim: bool,
    pub inverse: bool,
    pub hidden: bool,
}

impl Style {
    fn highlight(self, fallback: u32, canvas_background: u32) -> HighlightStyle {
        let mut foreground = self.foreground;
        let mut background = self.background;
        if self.inverse {
            (foreground, background) = (
                Some(background.unwrap_or(canvas_background)),
                Some(foreground.unwrap_or(fallback)),
            );
        }
        if self.hidden {
            foreground = Some(background.unwrap_or(canvas_background));
        }
        HighlightStyle {
            color: foreground.map(|color| {
                rgb(if self.hidden {
                    color
                } else {
                    display_color(color, background.unwrap_or(canvas_background))
                })
                .into()
            }),
            background_color: background.map(|color| rgb(color).into()),
            font_weight: self.bold.then_some(FontWeight::BOLD),
            font_style: self.italic.then_some(FontStyle::Italic),
            underline: self.underline.then_some(UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: false,
            }),
            strikethrough: self.strike.then_some(StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
            fade_out: self.dim.then_some(0.45),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct StyledLine {
    pub text: String,
    pub spans: Vec<(Range<usize>, Style)>,
}

impl StyledLine {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Prefix metadata is neutral; only the application's output gets ANSI styles.
    pub fn render(&self, prefix: &str, fallback: u32) -> StyledText {
        self.render_on(prefix, fallback, BACKGROUND)
    }

    /// Render against the actual surface, including inverse/hidden ANSI text.
    pub fn render_on(&self, prefix: &str, fallback: u32, background: u32) -> StyledText {
        let offset = prefix.len();
        StyledText::new(format!("{prefix}{}", self.text)).with_highlights(self.spans.iter().map(
            |(range, style)| {
                (
                    range.start + offset..range.end + offset,
                    style.highlight(fallback, background),
                )
            },
        ))
    }
}

/// Keep ANSI hues readable on light surfaces. Dark surfaces preserve the
/// original palette; application-specified dark ANSI backgrounds do too.
pub(crate) fn display_color(mut foreground: u32, background: u32) -> u32 {
    fn luminance(color: u32) -> f32 {
        let channel = |shift: u32| {
            let value = ((color >> shift) & 255u32) as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
    let background = luminance(background);
    if background <= 0.5 {
        return foreground;
    }
    while (background + 0.05) / (luminance(foreground) + 0.05) < 4.5 {
        let channel = |shift: u32| (((foreground >> shift) & 255u32) as f32 * 0.85) as u32;
        foreground = (channel(16) << 16) | (channel(8) << 8) | channel(0);
    }
    foreground
}

#[derive(Clone, Copy)]
struct Cell {
    character: char,
    style: Style,
}

#[derive(Default)]
struct Row {
    cells: Vec<Cell>,
    cursor: usize,
    bytes: usize,
    truncated: bool,
    prefix_len: usize,
    prefix_checked: bool,
}

impl Row {
    fn clear(&mut self) {
        self.cells.clear();
        self.cursor = 0;
        self.bytes = 0;
        self.truncated = false;
        self.prefix_len = 0;
        self.prefix_checked = false;
    }

    fn snapshot(&self) -> StyledLine {
        let mut line = StyledLine::default();
        for cell in &self.cells {
            let start = line.text.len();
            line.text.push(cell.character);
            if cell.style == Style::default() {
                continue;
            }
            if let Some((range, style)) = line.spans.last_mut()
                && *style == cell.style
                && range.end == start
            {
                range.end = line.text.len();
            } else if line.spans.len() < MAX_SPANS {
                line.spans.push((start..line.text.len(), cell.style));
            }
        }
        if self.truncated {
            line.text.push_str(" … [line truncated]");
        }
        line
    }

    fn recompute_bytes(&mut self) {
        self.bytes = self
            .cells
            .iter()
            .map(|cell| cell.character.len_utf8())
            .sum();
    }
}

/// Docker timestamps are metadata, not part of a service's ANSI stream.
pub fn split_timestamp(text: &str) -> (&str, &str) {
    if let Some((timestamp, _)) = text.split_once(' ')
        && chrono::DateTime::parse_from_rfc3339(timestamp).is_ok()
    {
        text.split_at(timestamp.len() + 1)
    } else {
        ("", text)
    }
}

#[derive(Default)]
struct State {
    row: Row,
    style: Style,
    completed: VecDeque<StyledLine>,
    dirty: bool,
    timestamp_prefixes: bool,
}

impl Handler for State {
    fn input(&mut self, character: char) {
        let row = &mut self.row;
        if row.truncated {
            return;
        }
        let padding = row.cursor.saturating_sub(row.cells.len());
        let old_bytes = row
            .cells
            .get(row.cursor)
            .map_or(0, |cell| cell.character.len_utf8());
        if row.bytes + padding + character.len_utf8() - old_bytes > MAX_BYTES {
            row.truncated = true;
            self.dirty = true;
            return;
        }
        row.cells.resize(
            row.cells.len() + padding,
            Cell {
                character: ' ',
                style: Style::default(),
            },
        );
        let cell = Cell {
            character,
            style: self.style,
        };
        if row.cursor < row.cells.len() {
            row.cells[row.cursor] = cell;
        } else {
            row.cells.push(cell);
        }
        row.bytes = row.bytes + padding + character.len_utf8() - old_bytes;
        row.cursor += 1;
        if self.timestamp_prefixes && !row.prefix_checked && character == ' ' {
            row.prefix_checked = true;
            let token = row.cells[..row.cursor - 1]
                .iter()
                .map(|cell| cell.character)
                .collect::<String>();
            if chrono::DateTime::parse_from_rfc3339(&token).is_ok() {
                row.prefix_len = row.cursor;
                for cell in &mut row.cells[..row.prefix_len] {
                    cell.style = Style::default();
                }
            }
        }
        self.dirty = true;
    }

    fn carriage_return(&mut self) {
        self.row.cursor = self.row.prefix_len;
    }
    fn backspace(&mut self) {
        self.row.cursor = self.row.cursor.saturating_sub(1).max(self.row.prefix_len);
    }
    fn goto_col(&mut self, col: usize) {
        self.row.cursor = self.row.prefix_len.saturating_add(col).min(MAX_BYTES);
    }
    fn move_forward(&mut self, cols: usize) {
        self.row.cursor = self.row.cursor.saturating_add(cols).min(MAX_BYTES);
    }
    fn move_backward(&mut self, cols: usize) {
        self.row.cursor = self
            .row
            .cursor
            .saturating_sub(cols)
            .max(self.row.prefix_len);
    }
    fn put_tab(&mut self, count: u16) {
        for _ in 0..count {
            let next = self.row.prefix_len + ((self.row.cursor - self.row.prefix_len) / 8 + 1) * 8;
            while self.row.cursor < next && !self.row.truncated {
                self.input(' ');
            }
        }
    }
    fn clear_line(&mut self, mode: LineClearMode) {
        match mode {
            LineClearMode::Right => self.row.cells.truncate(self.row.cursor),
            LineClearMode::Left => {
                let end = self.row.cursor.saturating_add(1).min(self.row.cells.len());
                self.row.cells[self.row.prefix_len..end].fill(Cell {
                    character: ' ',
                    style: Style::default(),
                });
            }
            LineClearMode::All => self.row.cells.truncate(self.row.prefix_len),
        }
        self.row.recompute_bytes();
        self.row.truncated = false;
        self.dirty = true;
    }
    fn linefeed(&mut self) {
        self.completed.push_back(self.row.snapshot());
        while self.completed.len() > crate::model::LogBuffer::MAX_LINES {
            self.completed.pop_front();
        }
        self.row.clear();
        self.dirty = true;
    }
    fn newline(&mut self) {
        self.linefeed();
    }
    fn reset_state(&mut self) {
        self.style = Style::default();
    }
    fn terminal_attribute(&mut self, attr: Attr) {
        match attr {
            Attr::Reset => self.style = Style::default(),
            Attr::Foreground(color) => self.style.foreground = color_rgb(color),
            Attr::Background(color) => self.style.background = color_rgb(color),
            Attr::Bold => self.style.bold = true,
            Attr::Dim => self.style.dim = true,
            Attr::Italic => self.style.italic = true,
            Attr::Underline
            | Attr::DoubleUnderline
            | Attr::Undercurl
            | Attr::DottedUnderline
            | Attr::DashedUnderline => self.style.underline = true,
            Attr::Strike => self.style.strike = true,
            Attr::Reverse => self.style.inverse = true,
            Attr::Hidden => self.style.hidden = true,
            Attr::CancelBold => self.style.bold = false,
            Attr::CancelBoldDim => {
                self.style.bold = false;
                self.style.dim = false;
            }
            Attr::CancelItalic => self.style.italic = false,
            Attr::CancelUnderline => self.style.underline = false,
            Attr::CancelStrike => self.style.strike = false,
            Attr::CancelReverse => self.style.inverse = false,
            Attr::CancelHidden => self.style.hidden = false,
            _ => {}
        }
    }
}

pub(crate) fn indexed_color(index: u8) -> u32 {
    match index {
        0..=15 => PALETTE[index as usize],
        232..=255 => {
            let value = 8 + u32::from(index - 232) * 10;
            (value << 16) | (value << 8) | value
        }
        _ => {
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

fn color_rgb(color: Color) -> Option<u32> {
    match color {
        Color::Spec(color) => {
            Some((u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b))
        }
        Color::Named(NamedColor::Foreground | NamedColor::Background) => None,
        Color::Named(color) => PALETTE.get(color as usize).copied().or(Some(FOREGROUND)),
        Color::Indexed(index) => Some(indexed_color(index)),
    }
}

/// One decoder per output stream. VTE retains split UTF-8 and escape sequences.
#[derive(Default)]
pub struct Decoder {
    parser: Processor,
    state: State,
}

impl Decoder {
    pub fn record(&mut self, text: &str) -> StyledLine {
        self.parser.advance(&mut self.state, text.as_bytes());
        let line = self.state.row.snapshot();
        self.state.row.clear();
        self.state.completed.clear();
        self.state.dirty = false;
        line
    }
}

#[derive(Default)]
pub struct Buffer {
    pub lines: VecDeque<StyledLine>,
    pub partial: StyledLine,
    decoder: Decoder,
}

impl Buffer {
    pub fn with_timestamps() -> Self {
        let mut buffer = Self::default();
        buffer.decoder.state.timestamp_prefixes = true;
        buffer
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.decoder.parser.advance(&mut self.decoder.state, bytes);
        self.lines.append(&mut self.decoder.state.completed);
        while self.lines.len() > crate::model::LogBuffer::MAX_LINES {
            self.lines.pop_front();
        }
        if self.decoder.state.dirty {
            self.partial = self.decoder.state.row.snapshot();
            self.decoder.state.dirty = false;
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .chain((!self.partial.is_empty()).then_some(self.partial.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_colors_remain_readable_and_hidden_text_uses_the_current_surface() {
        let green = PALETTE[2];
        assert_eq!(display_color(green, BACKGROUND), green);
        let light_green = display_color(green, 0xffffff);
        assert!(light_green < green);
        assert_eq!(display_color(0x161616, 0xffffff), 0x161616);
        let hidden = Style {
            hidden: true,
            ..Style::default()
        }
        .highlight(0x161616, 0xffffff);
        assert_eq!(hidden.color, Some(rgb(0xffffff).into()));
        let inverted = Style {
            inverse: true,
            ..Style::default()
        }
        .highlight(0x161616, 0xffffff);
        assert_eq!(inverted.color, Some(rgb(0xffffff).into()));
        assert_eq!(inverted.background_color, Some(rgb(0x161616).into()));
    }

    #[test]
    fn split_sequences_utf8_resets_and_plain_copy() {
        let mut buffer = Buffer::default();
        buffer.push(b"\x1b[3");
        buffer.push(b"2mwrite caf\xc3");
        buffer.push(b"\xa9\x1b[0m www/styles/main.css\n");
        assert_eq!(buffer.text(), "write café www/styles/main.css");
        assert_eq!(buffer.lines[0].spans[0].0, 0..11);
        assert_eq!(buffer.lines[0].spans[0].1.foreground, Some(PALETTE[2]));
        buffer.push(b"neutral\n");
        assert!(buffer.lines[1].spans.is_empty());
    }

    #[test]
    fn only_injected_timestamps_are_neutral_not_application_dates() {
        let timestamp = "2026-10-12T23:30:00.000000001Z";
        let mut container = Buffer::with_timestamps();
        container
            .push(format!("\x1b[32mfirst\n{timestamp} {timestamp} application date\n").as_bytes());
        assert_eq!(container.lines[1].spans[0].0.start, timestamp.len() + 1);
        let mut operation = Buffer::default();
        operation.push(format!("\x1b[32m{timestamp} application date\n").as_bytes());
        assert_eq!(operation.lines[0].spans[0].0.start, 0);
        let mut project = Decoder::default();
        let line = project.record(&format!("\x1b[32m{timestamp} application date"));
        assert_eq!(line.spans[0].0.start, 0);
        let mut progress = Buffer::with_timestamps();
        progress.push(format!("{timestamp} \x1b[32mbuilding 10%").as_bytes());
        progress.push(b"\r\x1b[2Kdone\x1b[0m\n");
        assert_eq!(progress.text(), format!("{timestamp} done"));
        assert_eq!(progress.lines[0].spans[0].0.start, timestamp.len() + 1);
    }

    #[test]
    fn progress_overwrites_and_erase_in_line_keep_one_record() {
        let mut buffer = Buffer::default();
        buffer.push(b"building 10%\r\x1b[2Kbuilding 20%");
        assert_eq!(buffer.partial.text, "building 20%");
        assert!(buffer.lines.is_empty());
        buffer.push(b"\r\x1b[Kdone\nabc\x08Z\n");
        assert_eq!(buffer.text(), "done\nabZ");
    }

    #[test]
    fn truecolor_indexed_attributes_and_osc_are_not_literal_text() {
        let mut decoder = Decoder::default();
        let line = decoder.record("\x1b[1;3;4;38;2;10;20;30;48;5;196mstyled\x1b[0m \x1b]8;;https://example.com\x07link\x1b]8;;\x1b\\");
        assert_eq!(line.text, "styled link");
        let style = line.spans[0].1;
        assert_eq!(style.foreground, Some(0x0a141e));
        assert_eq!(style.background, Some(0xff0000));
        assert!(style.bold && style.italic && style.underline);
        assert_eq!(line.spans[0].0, 0..6);
    }

    #[test]
    fn history_line_and_style_work_are_bounded() {
        let mut buffer = Buffer::default();
        buffer.push("x".repeat(MAX_BYTES + 100).as_bytes());
        assert!(buffer.partial.text.ends_with("[line truncated]"));
        buffer.push(b"\n");
        for _ in 0..2000 {
            buffer.push(b"line\n");
        }
        assert_eq!(buffer.lines.len(), crate::model::LogBuffer::MAX_LINES);
        let mut decoder = Decoder::default();
        let styled = decoder.record(&"\x1b[31mx\x1b[32my".repeat(1000));
        assert_eq!(styled.text.len(), 2000);
        assert_eq!(styled.spans.len(), MAX_SPANS);
    }
}
