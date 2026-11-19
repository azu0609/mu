use crate::{
    App, Result, markdown,
    text::{ACCENT, GRAY, Line, Span, Style, clean, clip, wrap},
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
use mu_client::{Block, Kind, Status, Tool, ToolStatus, counts};
use std::io::{self, Write};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub struct Terminal;

impl Terminal {
    pub fn enter() -> Result<Self> {
        let old = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            old(info);
        }));
        let mut guard = Self;
        guard.resume()?;
        Ok(guard)
    }

    pub fn suspend<T>(&mut self, run: impl FnOnce() -> T) -> Result<T> {
        restore();
        let result = run();
        self.resume()?;
        Ok(result)
    }

    fn resume(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            cursor::Hide
        )?;
        Ok(())
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
        LeaveAlternateScreen,
        terminal::SetTitle("")
    );
    let _ = terminal::disable_raw_mode();
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

pub fn title(app: &App) -> String {
    let name = app.view.cwd.file_name().unwrap_or_else(|| app.view.cwd.as_os_str());
    // OSC title strings must not contain terminal control characters (including ESC and BEL).
    let name: String = name.to_string_lossy().chars().filter(|c| !c.is_control()).collect();
    format!("{}mu · {name}", marker(app.view.state.status))
}

pub fn marker(status: Status) -> &'static str {
    match status {
        Status::Idle => "",
        Status::Running => "… ",
        Status::Finished => "✓ ",
        Status::Failed => "! ",
    }
}

pub fn set_title(title: &str) -> Result<()> {
    execute!(io::stdout(), terminal::SetTitle(title))?;
    Ok(())
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
            self.chars.remove(start - 1);
            self.cursor = start - 1;
        } else if end < self.chars.len() {
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

impl Line {
    fn print(&self, out: &mut impl Write, width: usize) -> Result<()> {
        let mut remaining = width;
        for span in &self.spans {
            if remaining == 0 {
                break;
            }
            let text = clean(&span.text).replace('\n', " ");
            let visible = clip(&text, remaining);
            queue!(out, SetAttribute(Attribute::Reset), SetForegroundColor(span.style.color))?;
            if span.style.bold {
                queue!(out, SetAttribute(Attribute::Bold))?;
            }
            if span.style.italic {
                queue!(out, SetAttribute(Attribute::Italic))?;
            }
            if span.style.underlined {
                queue!(out, SetAttribute(Attribute::Underlined))?;
            }
            queue!(out, Print(&visible))?;
            remaining = remaining.saturating_sub(visible.width());
            if visible.len() < text.len() {
                break;
            }
        }
        Ok(())
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
    let mut used = 1;
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

fn tool_lines(marker: &str, command: &str, tool: &Tool, width: usize, expanded: bool) -> Vec<Line> {
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
        vec![tail_ellipsis(&summary, command_width)]
    } else {
        vec![ellipsis(&summary, command_width)]
    };
    let mut result = vec![];
    for (i, line) in command.into_iter().enumerate() {
        let mut rendered = Line::new(if i == 0 { marker } else { " " }, if i == 0 { color } else { Color::Reset });
        rendered.push(&format!(" {line}"), Style::new(Color::Reset));
        result.push(rendered);
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
    } else if expanded && (hidden_input || lines.len() > 3) {
        result.push(Line::new("  │ Ctrl+O to collapse", GRAY));
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
    let (first_prefix, color, limit) = match (block.kind, &block.tool) {
        (Kind::Call, Some(tool)) => return tool_lines("●", &block.text, tool, width, expanded),
        (Kind::Shell, Some(tool)) => return tool_lines("$", &block.text, tool, width, expanded),
        (Kind::Call | Kind::Shell | Kind::Notice | Kind::Warning, _) => {
            let (marker, color) = if block.kind == Kind::Warning { ("!", Color::Yellow) } else { ("·", GRAY) };
            let mut lines = vec![];
            for (i, line) in wrap(&clean(&block.text), width.saturating_sub(4)).into_iter().enumerate() {
                lines.push(Line::new(format!("  {} {line}", if i == 0 { marker } else { " " }), color));
            }
            lines.push(Line::new("", GRAY));
            return lines;
        }
        (Kind::AgentMessage, _) => {
            let from =
                block.from.as_ref().map_or_else(String::new, |from| format!("{} · {}", from.title, from.session));
            let mut lines = vec![Line::new(format!("‹ {from}"), Color::Magenta)];
            for line in wrap(&clean(&block.text), width.saturating_sub(2)) {
                lines.push(Line::new(format!("  {line}"), Color::Reset));
            }
            lines.push(Line::new("", GRAY));
            return lines;
        }
        (Kind::User, _) => ("› ", ACCENT, usize::MAX),
        (Kind::Agent, _) => ("  ", Color::Reset, usize::MAX),
        (Kind::Thought, _) => ("  ", GRAY, 2),
    };
    if block.kind == Kind::Agent {
        let mut lines = markdown::render(&block.text, width.saturating_sub(2));
        for line in &mut lines {
            line.spans.insert(0, Span { text: "  ".into(), style: Style::new(Color::Reset) });
        }
        lines.push(Line::new("", GRAY));
        return lines;
    }
    let text = clean(&block.text);
    if text.is_empty() && block.kind == Kind::Thought {
        return vec![];
    }
    let mut lines = wrap(&text, width.saturating_sub(2));
    let collapsible = lines.len() > limit;
    let collapsed = !expanded && collapsible;
    if collapsed {
        lines.truncate(limit);
    }
    let mut result = vec![];
    let style = Style { italic: block.kind == Kind::Thought, ..Style::new(color) };
    for (i, line) in lines.into_iter().enumerate() {
        let prefix = if i == 0 { first_prefix } else { "  " };
        result.push(Line::styled(format!("{prefix}{line}"), style));
    }
    if collapsed {
        result.push(Line::styled("  … Ctrl+O to expand", style));
    } else if collapsible {
        result.push(Line::styled("  Ctrl+O to collapse", style));
    }
    result.push(Line::new("", GRAY));
    result
}

#[derive(Clone, Copy)]
pub struct ScrollAnchor {
    pub width: usize,
    pub expanded: bool,
    pub total: usize,
}

#[derive(Clone, Copy)]
pub struct CachedLines {
    pub width: usize,
    pub expanded: bool,
    pub blocks: usize,
    pub lines: usize,
}

impl CachedLines {
    fn matches(&self, width: usize, expanded: bool, blocks: usize) -> bool {
        self.width == width && self.expanded == expanded && self.blocks == blocks
    }
}

struct Transcript {
    pending: Vec<Block>,
}

impl Transcript {
    fn new(app: &App) -> Self {
        Self {
            pending: app
                .view
                .state
                .queued
                .iter()
                .enumerate()
                .map(|(i, text)| {
                    let hint = if i + 1 == app.view.state.queued.len() { "  [Ctrl+↑ restore]" } else { "" };
                    Block::new(Kind::Notice, format!("queued: {text}{hint}"))
                })
                .collect(),
        }
    }

    fn static_blocks<'a>(&'a self, app: &'a App) -> impl DoubleEndedIterator<Item = &'a Block> + 'a {
        app.view.blocks.iter()
    }

    fn dynamic_blocks<'a>(&'a self, app: &'a App) -> impl DoubleEndedIterator<Item = &'a Block> + 'a {
        app.view
            .live
            .blocks
            .iter()
            .chain(app.view.live.notice.iter())
            .chain(app.feedback.iter())
            .chain(app.shell.iter().map(|shell| &shell.block))
            .chain(self.pending.iter())
    }

    fn blocks<'a>(&'a self, app: &'a App) -> impl DoubleEndedIterator<Item = &'a Block> + 'a {
        self.static_blocks(app).chain(self.dynamic_blocks(app))
    }

    fn line_count(&self, app: &mut App, width: usize) -> usize {
        let blocks = app.view.blocks.len();
        let static_lines = match app.transcript_static {
            Some(cached) if cached.matches(width, app.expanded, blocks) => cached.lines,
            _ => {
                let lines = self
                    .static_blocks(app)
                    .map(|block| block_lines(block, width.saturating_sub(1), app.expanded).len())
                    .sum();
                app.transcript_static = Some(CachedLines { width, expanded: app.expanded, blocks, lines });
                lines
            }
        };
        static_lines
            + self
                .dynamic_blocks(app)
                .map(|block| block_lines(block, width.saturating_sub(1), app.expanded).len())
                .sum::<usize>()
    }
}

fn placeholder(app: &App) -> &'static str {
    if app.shell.is_some() {
        "Esc stops the command"
    } else if app.view.running() {
        "Queue a message · Esc to interrupt"
    } else if app.model().is_none() {
        "Pick a model with /model to start"
    } else {
        "Message mu · / commands & skills · @ files · ! run for the agent, !! just for you · Ctrl+G editor"
    }
}

pub fn transcript_line_count(app: &mut App, width: usize) -> usize {
    Transcript::new(app).line_count(app, width)
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
        let transcript = Transcript::new(app);
        if app.scroll > 0 {
            let total = transcript.line_count(app, width);
            if let Some(old) = app.scroll_layout
                && old.width == width
                && old.expanded == app.expanded
            {
                if total >= old.total {
                    app.scroll = app.scroll.saturating_add(total - old.total);
                } else {
                    app.scroll = app.scroll.saturating_sub(old.total - total);
                }
            }
            app.scroll = app.scroll.min(total.saturating_sub(transcript_height));
            app.scroll_layout = (app.scroll > 0).then_some(ScrollAnchor { width, expanded: app.expanded, total });
        } else {
            app.scroll_layout = None;
        }
        let need = transcript_height.saturating_add(app.scroll);
        let mut reversed = vec![];
        for block in transcript.blocks(app).rev() {
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
            line.print(&mut out, width.saturating_sub(1))?;
        }
    }
    if app.picker.is_none()
        && let Some(menu) = &app.completion
    {
        queue!(out, SetAttribute(Attribute::Reset))?;
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
        SetAttribute(Attribute::Reset),
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
        if text.is_empty() && line == 0 {
            queue!(out, SetForegroundColor(GRAY), Print(clip(placeholder(app), prompt_width)))?;
        } else if let Some(s) = input.get(line) {
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
    let model = app.model();
    let usage = &app.view.state.stats;
    let cache = usage.cached.map(counts::compact).unwrap_or_else(|| "?".into());
    let read = usage
        .cached
        .map(|c| format!("{:.1}%", 100.0 * c as f64 / usage.input.max(1) as f64))
        .unwrap_or_else(|| "?".into());
    let left = format!(
        " ↑{} ↓{} | cache {} read {} | ctx {}/{} ",
        counts::compact(usage.uncached_input),
        counts::compact(usage.output),
        cache,
        read,
        counts::compact(usage.context_used),
        model.and_then(|model| model.context).map(counts::compact).unwrap_or_else(|| "?".into())
    );
    let right = clip(
        &format!(
            "{}{}{} ",
            if app.view.running() { "· " } else { "" },
            model.map_or("no model · /model", |model| &model.name),
            model.and_then(|model| model.effort.as_ref()).map(|e| format!(" {e}")).unwrap_or_default()
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
