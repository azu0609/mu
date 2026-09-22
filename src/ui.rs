use crate::{
    App, Result,
    session::{Block, Kind},
};
use crossterm::{
    cursor,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
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
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture, cursor::Hide)?;
        Ok(guard)
    }
}
fn restore() {
    let _ = execute!(
        io::stdout(),
        ResetColor,
        cursor::Show,
        DisableMouseCapture,
        DisableBracketedPaste,
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
}
impl Editor {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }
    pub fn insert(&mut self, text: &str) {
        for ch in clean(text).chars() {
            self.chars.insert(self.cursor, ch);
            self.cursor += 1;
        }
    }
    pub fn replace(&mut self, range: std::ops::Range<usize>, text: &str) {
        self.cursor = range.start + text.chars().count();
        self.chars.splice(range, text.chars());
    }
    pub fn take(&mut self) -> String {
        let s = self.text();
        self.chars.clear();
        self.cursor = 0;
        s
    }
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }
    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }
    pub fn home(&mut self) {
        while self.cursor > 0 && self.chars[self.cursor - 1] != '\n' {
            self.cursor -= 1;
        }
    }
    pub fn end(&mut self) {
        while self.cursor < self.chars.len() && self.chars[self.cursor] != '\n' {
            self.cursor += 1;
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

// Compact counts for the status line: 999, 1.0k, 15.5k, 1.0M.
fn count(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 999_950 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

type Line = (String, Color);
fn block_lines(block: &Block, width: usize, expanded: bool) -> Vec<Line> {
    let (label, color) = match block.kind {
        Kind::User => ("›", ACCENT),
        Kind::Agent => ("µ", Color::White),
        Kind::Thought => ("· thoughts", GRAY),
        Kind::Call => ("$", ACCENT),
        Kind::Output => ("│", GRAY),
        Kind::Notice => ("!", Color::Yellow),
    };
    let text = clean(&block.text);
    if block.kind == Kind::Notice {
        return wrap(&format!("! {text}"), width).into_iter().map(|line| (line, color)).collect();
    }
    if text.is_empty() && block.kind == Kind::Thought {
        return vec![];
    }
    let mut lines = wrap(&text, width.saturating_sub(2));
    let limit = match block.kind {
        Kind::Thought => 2,
        Kind::Call | Kind::Output => 6,
        _ => usize::MAX,
    };
    let collapsed = !expanded && lines.len() > limit;
    if collapsed {
        lines.truncate(limit);
    }
    let mut result = vec![(label.into(), color)];
    let mut code = false;
    for line in lines {
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
        result.push((format!("  {line}"), md_color));
    }
    if collapsed {
        result.push(("  … Ctrl+O to expand".into(), GRAY));
    }
    result.push((String::new(), GRAY));
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
    let prompt_width = width - 2;
    let cursor_lines = wrap(&format!("{prefix} "), prompt_width);
    let cursor_row = cursor_lines.len() - 1;
    let cursor_col = cursor_lines.last().unwrap().width().saturating_sub(1);
    let mut input = wrap(&format!("{text} "), prompt_width);
    let input_height = input.len().min(6).min(h as usize - 4).max(1);
    let input_top = (cursor_row + 1).saturating_sub(input_height);
    let transcript_height = h as usize - input_height - 3;
    if text.is_empty() {
        input[0] = if app.worker.is_some() {
            "message to steer · Esc to stop"
        } else {
            "message · /model /new /resume /tree /copy /quit"
        }
        .into();
    }
    let mut out = io::BufWriter::new(io::stdout().lock());
    queue!(out, cursor::Hide, cursor::MoveTo(0, 0))?;
    let lines = if let Some(picker) = &app.picker {
        let mut lines = vec![(format!("{} · ↑/↓ Enter · Esc", picker.title), ACCENT)];
        let start = (picker.selected + 1).saturating_sub(transcript_height.saturating_sub(1));
        for (i, (_, label)) in picker.entries.iter().enumerate().skip(start).take(transcript_height.saturating_sub(1)) {
            lines.push((
                format!("{} {}", if i == picker.selected { "›" } else { " " }, label),
                if i == picker.selected { ACCENT } else { GRAY },
            ));
        }
        lines
    } else {
        let path = app.session.path();
        let pending: Vec<_> =
            app.queued.iter().map(|s| Block::new(Kind::Notice, format!("queued: {}", s.text))).collect();
        let welcome = [Block::new(Kind::Notice, "mu · µ · 無    Ctrl+O expand · PgUp/PgDn scroll")];
        let blocks: Vec<_> = welcome
            .iter()
            .chain(path.iter().flat_map(|&i| app.session.nodes[i].blocks.iter()))
            .chain(app.live.iter())
            .chain(app.notices.iter())
            .chain(pending.iter())
            .collect();
        let need = transcript_height.saturating_add(app.scroll);
        let mut reversed = vec![];
        for block in blocks.into_iter().rev() {
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
        if let Some((s, color)) = lines.get(row) {
            queue!(out, SetForegroundColor(*color), Print(clip(s, width.saturating_sub(1))))?;
        }
    }
    if app.picker.is_none()
        && let Some(menu) = &app.completion
    {
        let height = (menu.entries.len() + 1).min(7).min(transcript_height);
        let top = transcript_height - height;
        let start = (menu.selected + 1).saturating_sub(height.saturating_sub(1));
        for row in 0..height {
            queue!(out, cursor::MoveTo(0, (top + row) as u16), Clear(ClearType::CurrentLine))?;
            let (text, color) = if row == 0 {
                (
                    format!(
                        "{} · ↑/↓ Tab · Enter · Esc{}",
                        if menu.file { "files" } else { "commands / skills" },
                        if app.files.pending() {
                            " · searching…"
                        } else if menu.entries.is_empty() {
                            " · no matches"
                        } else {
                            ""
                        }
                    ),
                    GRAY,
                )
            } else {
                let i = start + row - 1;
                (
                    format!("{} {}", if i == menu.selected { "›" } else { " " }, menu.entries[i].label),
                    if i == menu.selected { ACCENT } else { GRAY },
                )
            };
            queue!(out, SetForegroundColor(color), Print(clip(&text, width - 1)))?;
        }
    }
    queue!(
        out,
        cursor::MoveTo(0, transcript_height as u16),
        SetForegroundColor(GRAY),
        Clear(ClearType::CurrentLine),
        Print("─".repeat(width - 1))
    )?;
    for row in 0..input_height {
        queue!(
            out,
            cursor::MoveTo(0, (transcript_height + 1 + row) as u16),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(if text.is_empty() { GRAY } else { Color::White }),
            Print(" ")
        )?;
        if let Some(s) = input.get(input_top + row) {
            queue!(out, Print(clip(s, prompt_width)))?;
        }
    }
    queue!(
        out,
        cursor::MoveTo(0, h - 2),
        SetForegroundColor(GRAY),
        Clear(ClearType::CurrentLine),
        Print("─".repeat(width - 1))
    )?;
    let usage = app.session.usage();
    let cache = usage.cached.map(count).unwrap_or_else(|| "?".into());
    let read = usage
        .cached
        .map(|c| format!("{:.1}%", 100.0 * c as f64 / usage.input.max(1) as f64))
        .unwrap_or_else(|| "?".into());
    let left = format!(
        " ↑{} ↓{} | cache {} read {} | ctx {}/{} ",
        count(usage.input),
        count(usage.output),
        cache,
        read,
        count(usage.input + usage.output),
        count(app.context)
    );
    let right = clip(
        &format!(
            "{}{}{} ",
            if app.worker.is_some() { "· " } else { "" },
            app.session.model,
            app.session.effort.as_ref().map(|e| format!(" {e}")).unwrap_or_default()
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
        ResetColor
    )?;
    if app.picker.is_none() {
        queue!(
            out,
            cursor::MoveTo((1 + cursor_col) as u16, (transcript_height + 1 + cursor_row - input_top) as u16),
            cursor::Show
        )?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_scale() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000), "1.0k");
        assert_eq!(count(1_100), "1.1k");
        assert_eq!(count(127_432), "127.4k");
        assert_eq!(count(999_949), "999.9k");
        assert_eq!(count(999_950), "1.0M");
        assert_eq!(count(1_000_000), "1.0M");
        assert_eq!(count(2_400_000), "2.4M");
    }

    #[test]
    fn controls_and_unicode() {
        assert_eq!(clean("a\x1b[31mb\x1b[0m\x1b]52;c;evil\x07c\r\n"), "abc\n");
        assert_eq!(wrap("無µa", 3), vec!["無µ", "a"]);
        let mut e = Editor::default();
        e.insert("無µ");
        e.cursor = 1;
        e.backspace();
        assert_eq!(e.text(), "µ");
    }
}
