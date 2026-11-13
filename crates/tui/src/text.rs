use crossterm::style::Color;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) const GRAY: Color = Color::DarkGrey;
pub(crate) const ACCENT: Color = Color::Cyan;

// Never replay terminal control sequences supplied by a tool or a model.
pub(crate) fn clean(text: &str) -> String {
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

pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
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

pub(crate) fn clip(s: &str, width: usize) -> String {
    let mut text = clean(s).replace('\n', " ");
    if text.width() > width {
        let end = text
            .char_indices()
            .map(|(i, ch)| i + ch.len_utf8())
            .take_while(|&end| text[..end].width() <= width)
            .last()
            .unwrap_or(0);
        text.truncate(end);
    }
    text
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Style {
    pub(crate) color: Color,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underlined: bool,
}

impl Style {
    pub(crate) fn new(color: Color) -> Self {
        Self { color, bold: false, italic: false, underlined: false }
    }
}

pub(crate) struct Span {
    pub(crate) text: String,
    pub(crate) style: Style,
}

pub(crate) struct Line {
    pub(crate) spans: Vec<Span>,
}

impl Line {
    pub(crate) fn new(text: impl Into<String>, color: Color) -> Self {
        Self::styled(text, Style::new(color))
    }

    pub(crate) fn styled(text: impl Into<String>, style: Style) -> Self {
        Self { spans: vec![Span { text: text.into(), style }] }
    }

    pub(crate) fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.style == style
        {
            last.text.push_str(text);
        } else {
            self.spans.push(Span { text: text.into(), style });
        }
    }
}

// Carry styles with the text, rather than rediscovering Markdown on each
// screen row. Delimiters and styled ranges can themselves cross a soft wrap.
pub(crate) fn wrap_spans(spans: Vec<Span>, width: usize) -> Vec<Line> {
    let width = width.max(1);
    if !spans.iter().any(|span| span.text.contains('\n'))
        && spans.iter().map(|span| span.text.width()).sum::<usize>() <= width
    {
        return vec![Line { spans }];
    }
    let mut lines = vec![];
    let mut line = Line { spans: vec![] };
    let mut col = 0;
    for span in spans {
        let mut part = String::new();
        let mut start = col;
        for ch in span.text.chars() {
            let end = part.len();
            part.push(ch);
            if ch == '\n' || (start + part.width() > width && col > 0) {
                part.truncate(end);
                line.push(&part, span.style);
                part.clear();
                lines.push(line);
                line = Line { spans: vec![] };
                start = 0;
                if ch != '\n' {
                    part.push(ch);
                }
            }
            if part.width() > width {
                part.clear();
            }
            col = start + part.width();
        }
        line.push(&part, span.style);
    }
    lines.push(line);
    lines
}
