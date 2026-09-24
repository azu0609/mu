use crate::{
    App, Result, counts,
    session::{Block, Kind, Tool, ToolStatus},
};
use crossterm::{
    cursor,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const GRAY: Color = Color::DarkGrey;
const ACCENT: Color = Color::Cyan;

pub struct Terminal;
impl Terminal {
    pub fn enter() -> Result<Self> {
        let old = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            old(info);
        }));
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            cursor::Hide
        )?;
        Ok(guard)
    }
}
fn restore() {
    let _ = execute!(
        io::stdout(),
        ResetColor,
        SetAttribute(Attribute::Reset),
        cursor::Show,
        DisableMouseCapture,
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}
impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

#[derive(Default)]
pub struct Editor {
    pub chars: Vec<char>,
    pub cursor: usize,
    preferred_col: Option<usize>,
}
impl Editor {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn insert(&mut self, text: &str) {
        self.replace(self.cursor..self.cursor, &clean(text));
    }
    pub fn replace(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.cursor = range.start + text.chars().count();
        self.chars.splice(range, text.chars());
        self.preferred_col = None;
    }
    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
        self.preferred_col = None;
    }
    pub fn clear_line(&mut self) {
        let start = self.chars[..self.cursor].iter().rposition(|&ch| ch == '\n').map_or(0, |i| i + 1);
        let end =
            self.chars[self.cursor..].iter().position(|&ch| ch == '\n').map_or(self.chars.len(), |i| self.cursor + i);
        if start < end {
            self.chars.drain(start..end);
            self.cursor = start;
        } else if start > 0 {
            // An empty line: remove the preceding newline and move to the
            // previous line, so another Ctrl+U can clear it too.
            self.chars.remove(start - 1);
            self.cursor = start - 1;
        } else if end < self.chars.len() {
            // The first line has no preceding newline; join it with the next.
            self.chars.remove(end);
            self.cursor = 0;
        }
        self.preferred_col = None;
    }
    pub fn backspace(&mut self) {
        self.preferred_col = None;
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }
    pub fn delete(&mut self) {
        self.preferred_col = None;
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }
    pub fn home(&mut self) {
        self.preferred_col = None;
        while self.cursor > 0 && self.chars[self.cursor - 1] != '\n' {
            self.cursor -= 1;
        }
    }
    pub fn end(&mut self) {
        self.preferred_col = None;
        while self.cursor < self.chars.len() && self.chars[self.cursor] != '\n' {
            self.cursor += 1;
        }
    }
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.preferred_col = None;
    }
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
        self.preferred_col = None;
    }
    // Each insertion point's screen row and column, using the same character
    // wrapping as the rendered input (including its trailing cursor space).
    fn positions(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut positions = Vec::with_capacity(self.chars.len() + 1);
        let (mut row, mut col) = (0, 0);
        for ch in self.chars.iter().copied().map(Some).chain(std::iter::once(None)) {
            positions.push(if col == width { (row + 1, 0) } else { (row, col) });
            match ch {
                Some('\n') => {
                    row += 1;
                    col = 0;
                }
                Some(ch) => {
                    let char_width = ch.width().unwrap_or(0);
                    if char_width <= width {
                        if col + char_width > width && col > 0 {
                            row += 1;
                            col = 0;
                        }
                        col += char_width;
                    }
                }
                None => (),
            }
        }
        positions
    }
    pub fn move_vertical(&mut self, width: usize, up: bool) {
        let positions = self.positions(width);
        let (row, col) = positions[self.cursor];
        let target = if up { row.checked_sub(1) } else { row.checked_add(1) };
        let Some(target) = target else { return };
        let preferred = self.preferred_col.unwrap_or(col);
        let best = positions
            .iter()
            .enumerate()
            .filter(|(_, (r, _))| *r == target)
            .min_by_key(|(index, (_, c))| (c.abs_diff(preferred), std::cmp::Reverse(*index)));
        if let Some((index, _)) = best {
            self.cursor = index;
            self.preferred_col = Some(preferred);
        }
    }
    pub fn word_backspace(&mut self) {
        while self.cursor > 0 && self.chars[self.cursor - 1].is_whitespace() {
            self.backspace();
        }
        while self.cursor > 0 && !self.chars[self.cursor - 1].is_whitespace() {
            self.backspace();
        }
    }
}

// Never replay terminal control sequences supplied by a tool or a model.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('_') | Some('^') => {
                    let mut esc = false;
                    for c in chars.by_ref() {
                        if c == '\x07' || (esc && c == '\\') {
                            break;
                        }
                        esc = c == '\x1b';
                    }
                }
                _ => (),
            }
        } else if ch == '\t' {
            out.push_str("    ");
        } else if ch == '\n' || !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    for line in text.split('\n') {
        let mut current = String::new();
        let mut col = 0;
        for ch in line.chars() {
            let w = ch.width().unwrap_or(0);
            if col + w > width && !current.is_empty() {
                lines.push(current);
                current = String::new();
                col = 0;
            }
            if w <= width {
                current.push(ch);
                col += w;
            }
        }
        lines.push(current);
    }
    lines
}
fn clip(s: &str, width: usize) -> String {
    wrap(&clean(s).replace('\n', " "), width).into_iter().next().unwrap_or_default()
}

struct Line {
    text: String,
    color: Color,
    bullet: Option<Color>,
    italic: bool,
}
impl Line {
    fn new(text: impl Into<String>, color: Color) -> Self {
        Self { text: text.into(), color, bullet: None, italic: false }
    }
}

fn ellipsis(text: &str, width: usize) -> String {
    if text.width() <= width {
        text.into()
    } else if width <= 1 {
        "…".repeat(width)
    } else {
        format!("{}…", clip(text, width - 1))
    }
}

fn tail_ellipsis(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    if width <= 1 {
        return "…".repeat(width);
    }
    let mut start = text.len();
    let mut used = 1; // Leading ellipsis.
    for (i, ch) in text.char_indices().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        start = i;
    }
    format!("…{}", &text[start..])
}

fn tool_lines(command: &str, tool: &Tool, width: usize, expanded: bool) -> Vec<Line> {
    let color = match tool.status {
        ToolStatus::Running | ToolStatus::TimedOut(_) | ToolStatus::Cancelled => Color::Yellow,
        ToolStatus::Exited(0) => Color::Green,
        ToolStatus::Exited(_) | ToolStatus::Error => Color::Red,
        ToolStatus::Pending => GRAY,
    };
    let text = clean(command);
    let command_width = width.saturating_sub(2);
    let summary = text.lines().map(str::trim).collect::<Vec<_>>().join(" ↵ ");
    let hidden_input = text.contains('\n') || summary.width() > command_width;
    let command = if expanded {
        wrap(&text, command_width)
    } else if matches!(tool.status, ToolStatus::Pending) {
        // Follow the newest input on one line while the model is generating it.
        vec![tail_ellipsis(&summary, command_width)]
    } else {
        vec![ellipsis(&summary, command_width)]
    };
    let mut result = vec![];
    for (i, line) in command.into_iter().enumerate() {
        result.push(Line {
            text: format!("{}{line}", if i == 0 { "● " } else { "  " }),
            color: Color::Reset,
            bullet: (i == 0).then_some(color),
            italic: false,
        });
    }
    let output = clean(&tool.output.text);
    let output = output.trim_end_matches('\n');
    let lines = if output.is_empty() { vec![] } else { wrap(output, width.saturating_sub(4)) };
    let hidden_output = !expanded && lines.len() > 3;
    let start = if expanded { 0 } else { lines.len().saturating_sub(3) };
    if hidden_output {
        result.push(Line::new(
            format!(
                "  │ … {start} {}{} hidden · Ctrl+O to expand",
                if tool.output.truncated { "preview " } else { "" },
                if start == 1 { "line" } else { "lines" }
            ),
            GRAY,
        ));
    } else if !expanded && hidden_input {
        result.push(Line::new("  │ … Ctrl+O to expand", GRAY));
    }
    for line in &lines[start..] {
        result.push(Line::new(format!("  │ {line}"), GRAY));
    }
    let mut footer: Vec<_> = tool.status.summary().into_iter().collect();
    if tool.output.truncated {
        footer.push("preview only; overflow in temp file".into());
    }
    let mut footer = footer.join(" · ");
    if expanded
        && tool.output.truncated
        && let Some(log) = &tool.output.log
    {
        footer.push_str(&format!("\noutput log (temporary): {}", clean(&log.to_string_lossy())));
    }
    if !footer.is_empty() {
        let footer_width = width.saturating_sub(4);
        let lines = if expanded { wrap(&footer, footer_width) } else { vec![ellipsis(&footer, footer_width)] };
        for (i, line) in lines.into_iter().enumerate() {
            result.push(Line::new(format!("  {} {line}", if i == 0 { "└" } else { " " }), GRAY));
        }
    }
    result.push(Line::new("", GRAY));
    result
}

fn block_lines(block: &Block, width: usize, expanded: bool) -> Vec<Line> {
    let (first_prefix, color, limit) = match block.kind {
        Kind::Call => return tool_lines(&block.text, block.tool.as_ref().unwrap(), width, expanded),
        Kind::Notice => {
            return wrap(&format!("! {}", clean(&block.text)), width)
                .into_iter()
                .map(|line| Line::new(line, Color::Yellow))
                .collect();
        }
        Kind::User => ("› ", ACCENT, usize::MAX),
        Kind::Agent => ("  ", Color::Reset, usize::MAX),
        Kind::Thought => ("  ", GRAY, 2),
    };
    let text = clean(&block.text);
    if text.is_empty() && block.kind == Kind::Thought {
        return vec![];
    }
    let mut lines = wrap(&text, width.saturating_sub(2));
    let collapsed = !expanded && lines.len() > limit;
    if collapsed {
        lines.truncate(limit);
    }
    let mut result = vec![];
    let mut code = false;
    for (i, line) in lines.into_iter().enumerate() {
        let trimmed = line.trim_start();
        let md_color = if block.kind == Kind::Agent {
            if trimmed.starts_with("```") {
                code = !code;
                GRAY
            } else if code || trimmed.starts_with('#') || trimmed.starts_with("> ") {
                ACCENT
            } else {
                color
            }
        } else {
            color
        };
        let prefix = if i == 0 { first_prefix } else { "  " };
        let mut rendered = Line::new(format!("{prefix}{line}"), md_color);
        rendered.italic = block.kind == Kind::Thought;
        result.push(rendered);
    }
    if collapsed {
        let mut line = Line::new("  … Ctrl+O to expand", GRAY);
        line.italic = block.kind == Kind::Thought;
        result.push(line);
    }
    result.push(Line::new("", GRAY));
    result
}

pub fn draw(app: &mut App) -> Result<()> {
    let (w, h) = terminal::size()?;
    let width = w as usize;
    if w < 4 || h < 5 {
        return Ok(());
    }
    let text = app.editor.text();
    let prefix: String = app.editor.chars[..app.editor.cursor].iter().collect();
    let prompt_width = width - 3;
    let cursor_lines = wrap(&format!("{prefix} "), prompt_width);
    let cursor_row = cursor_lines.len() - 1;
    let cursor_col = cursor_lines.last().unwrap().width().saturating_sub(1);
    let input = wrap(&format!("{text} "), prompt_width);
    let input_height = input.len().min(6).min(h as usize - 4).max(1);
    let input_top = (cursor_row + 1).saturating_sub(input_height);
    let transcript_height = h as usize - input_height - 3;
    let mut out = io::BufWriter::new(io::stdout().lock());
    queue!(out, cursor::Hide, cursor::MoveTo(0, 0))?;
    let lines = if let Some(picker) = &app.picker {
        let mut lines = vec![Line::new(picker.title, ACCENT)];
        let start = (picker.selected + 1).saturating_sub(transcript_height.saturating_sub(1));
        for (i, (_, label)) in picker.entries.iter().enumerate().skip(start).take(transcript_height.saturating_sub(1)) {
            lines.push(Line::new(
                format!("{} {}", if i == picker.selected { "›" } else { " " }, label),
                if i == picker.selected { ACCENT } else { GRAY },
            ));
        }
        lines
    } else {
        let path = app.session.path();
        let pending: Vec<_> =
            app.queued.iter().map(|s| Block::new(Kind::Notice, format!("queued: {}", s.text))).collect();
        let welcome = [Block::new(Kind::Notice, "mu · Ctrl+O to expand/collapse · PgUp/PgDn to scroll")];
        let blocks = welcome
            .iter()
            .chain(path.iter().flat_map(|&i| app.session.node(i).blocks.iter()))
            .chain(app.live.iter())
            .chain(app.notices.iter())
            .chain(pending.iter());
        let need = transcript_height.saturating_add(app.scroll);
        let mut reversed = vec![];
        for block in blocks.rev() {
            reversed.extend(block_lines(block, width - 1, app.expanded).into_iter().rev());
            if reversed.len() >= need {
                break;
            }
        }
        app.scroll = app.scroll.min(reversed.len().saturating_sub(transcript_height));
        let mut lines: Vec<_> = reversed.into_iter().skip(app.scroll).take(transcript_height).collect();
        lines.reverse();
        lines
    };
    for row in 0..transcript_height {
        queue!(out, cursor::MoveTo(0, row as u16), Clear(ClearType::CurrentLine))?;
        if let Some(line) = lines.get(row) {
            let text = clip(&line.text, width.saturating_sub(1));
            queue!(out, SetAttribute(if line.italic { Attribute::Italic } else { Attribute::NoItalic }))?;
            if let Some(color) = line.bullet {
                queue!(
                    out,
                    SetForegroundColor(color),
                    Print("●"),
                    SetForegroundColor(line.color),
                    Print(&text["●".len()..])
                )?;
            } else {
                queue!(out, SetForegroundColor(line.color), Print(text))?;
            }
        }
    }
    if app.picker.is_none()
        && let Some(menu) = &app.completion
    {
        queue!(out, SetAttribute(Attribute::NoItalic))?;
        let height = menu.entries.len().min(7).min(transcript_height);
        let top = transcript_height - height;
        let start = (menu.selected + 1).saturating_sub(height);
        for row in 0..height {
            queue!(out, cursor::MoveTo(0, (top + row) as u16), Clear(ClearType::CurrentLine))?;
            let i = start + row;
            let text = format!("{} {}", if i == menu.selected { "›" } else { " " }, menu.entries[i].label);
            let color = if i == menu.selected { ACCENT } else { GRAY };
            queue!(out, SetForegroundColor(color), Print(clip(&text, width - 1)))?;
        }
    }
    queue!(
        out,
        SetAttribute(Attribute::NoItalic),
        cursor::MoveTo(0, transcript_height as u16),
        SetForegroundColor(GRAY),
        Clear(ClearType::CurrentLine),
        Print("─".repeat(width))
    )?;
    for row in 0..input_height {
        let line = input_top + row;
        queue!(
            out,
            cursor::MoveTo(0, (transcript_height + 1 + row) as u16),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(if line == 0 { ACCENT } else { Color::Reset }),
            Print(if line == 0 { "› " } else { "  " }),
            SetForegroundColor(Color::Reset)
        )?;
        if let Some(s) = input.get(line) {
            queue!(out, Print(clip(s, prompt_width)))?;
        }
    }
    queue!(
        out,
        cursor::MoveTo(0, h - 2),
        SetForegroundColor(GRAY),
        Clear(ClearType::CurrentLine),
        Print("─".repeat(width))
    )?;
    let model = app.session.model();
    let usage = app.session.total_usage();
    let context = app.session.usage();
    let cache = usage.cached.map(counts::compact).unwrap_or_else(|| "?".into());
    let read = usage
        .cached
        .map(|c| format!("{:.1}%", 100.0 * c as f64 / usage.input.max(1) as f64))
        .unwrap_or_else(|| "?".into());
    let left = format!(
        " ↑{} ↓{} | cache {} read {} | ctx {}/{} ",
        counts::compact(app.session.uncached_input()),
        counts::compact(usage.output),
        cache,
        read,
        counts::compact(context.input.saturating_add(context.output)),
        counts::compact(model.context)
    );
    let right = clip(
        &format!(
            "{}{}{} ",
            if app.worker.is_some() { "· " } else { "" },
            model.name,
            model.effort.as_ref().map(|e| format!(" {e}")).unwrap_or_default()
        ),
        width.saturating_sub(1),
    );
    let room = width.saturating_sub(right.width() + 1);
    queue!(out, cursor::MoveTo(0, h - 1), Clear(ClearType::CurrentLine))?;
    for ch in clip(&left, room.max(1)).chars().take(if room == 0 { 0 } else { usize::MAX }) {
        queue!(out, SetForegroundColor(if ch.is_ascii_digit() || ch == '.' { ACCENT } else { GRAY }), Print(ch))?;
    }
    queue!(
        out,
        cursor::MoveTo((width - right.width()) as u16, h - 1),
        SetForegroundColor(GRAY),
        Print(right),
        ResetColor,
        SetAttribute(Attribute::Reset)
    )?;
    if app.picker.is_none() {
        queue!(
            out,
            cursor::MoveTo((2 + cursor_col) as u16, (transcript_height + 1 + cursor_row - input_top) as u16),
            cursor::Show
        )?;
    }
    out.flush()?;
    Ok(())
}
