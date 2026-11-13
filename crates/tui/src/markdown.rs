use crate::text::{ACCENT, GRAY, Line, Span, Style, clean, wrap_spans};
use crossterm::style::Color;
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

struct Table {
    range: Range<usize>,
    alignment: Vec<Alignment>,
    rows: Vec<Vec<Range<usize>>>,
}

struct Source<'a> {
    text: &'a str,
    styles: Vec<(Range<usize>, Style)>,
    cursor: usize,
}

impl Source<'_> {
    fn highlight_to(&mut self, end: usize, style: Style) {
        if end > self.cursor {
            self.styles.push((self.cursor..end, style));
            self.cursor = end;
        }
    }

    fn spans(&self, range: Range<usize>) -> Vec<Span> {
        let mut line = Line { spans: vec![] };
        let start = self.styles.partition_point(|(span, _)| span.end <= range.start);
        for (span, style) in self.styles[start..].iter().take_while(|(span, _)| span.start < range.end) {
            line.push(&self.text[span.start.max(range.start)..span.end.min(range.end)], *style);
        }
        line.spans
    }
}

// Use source offsets to keep markers visible without recognizing syntax ourselves.
// Nested styles compose on a stack.
fn parse(text: &str) -> (Source<'_>, Vec<Table>) {
    let plain = Style::new(Color::Reset);
    let mut source = Source { text, styles: vec![], cursor: 0 };
    let mut stack = vec![];
    let mut tables = vec![];
    let mut table: Option<Table> = None;
    for (event, range) in Parser::new_ext(text, Options::ENABLE_TABLES).into_offset_iter() {
        let (parent, base) = stack.last().copied().unwrap_or((TagEnd::Paragraph, plain));
        let accent = Style { color: ACCENT, bold: true, ..base };
        match event {
            Event::Start(tag) => {
                source.highlight_to(range.start, if parent == TagEnd::Item { accent } else { base });
                let style = match &tag {
                    Tag::Heading { .. } | Tag::Strong | Tag::TableHead => accent,
                    Tag::BlockQuote(_) => Style { color: ACCENT, italic: true, ..base },
                    Tag::Emphasis => Style { italic: true, ..base },
                    Tag::Link { .. } | Tag::Image { .. } => Style { color: ACCENT, underlined: true, ..base },
                    Tag::CodeBlock(_) => Style::new(GRAY),
                    _ => base,
                };
                match &tag {
                    // Nested tables stay source-visible so list/quote prefixes aren't lost.
                    Tag::Table(alignment) if stack.is_empty() => {
                        table = Some(Table { range, alignment: alignment.clone(), rows: vec![] });
                    }
                    Tag::TableHead | Tag::TableRow => {
                        if let Some(table) = &mut table {
                            table.rows.push(vec![]);
                        }
                    }
                    Tag::TableCell => {
                        if let Some(table) = &mut table {
                            table.rows.last_mut().unwrap().push(range);
                        }
                    }
                    _ => (),
                }
                stack.push((tag.to_end(), style));
            }
            Event::End(tag) => {
                source.highlight_to(range.end, base);
                stack.pop();
                if tag == TagEnd::Table
                    && let Some(table) = table.take()
                {
                    tables.push(table);
                }
            }
            event => {
                source.highlight_to(range.start, if parent == TagEnd::Item { accent } else { base });
                let style = match event {
                    Event::Code(_) => Style { color: Color::Yellow, ..base },
                    Event::Text(_) if parent == TagEnd::CodeBlock => Style::new(Color::Green),
                    _ => base,
                };
                source.highlight_to(range.end, style);
            }
        }
    }
    source.highlight_to(text.len(), plain);
    (source, tables)
}

fn table_lines(source: &Source<'_>, table: &Table, width: usize) -> Option<Vec<Line>> {
    let columns = table.alignment.len();
    // If even two characters per cell cannot fit, show the wrapped source.
    if columns == 0 || columns * 2 + (columns - 1) * 3 > width {
        return None;
    }
    let rows: Vec<Vec<Vec<Span>>> = table
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|range| {
                    let text = &source.text[range.clone()];
                    let start = range.start + text.len() - text.trim_start().len();
                    let mut spans = source.spans(start..start + text.trim().len());
                    // GFM table escapes aren't part of the displayed cell, even in code spans.
                    for span in &mut spans {
                        span.text = span.text.replace("\\|", "|");
                    }
                    spans
                })
                .collect()
        })
        .collect();
    let available = width - (columns - 1) * 3;
    let mut sizes: Vec<_> = (0..columns)
        .map(|col| {
            rows.iter()
                .map(|row| row[col].iter().map(|span| span.text.width()).sum::<usize>())
                .max()
                .unwrap_or(0)
                .max(2)
                .min(available)
        })
        .collect();
    // Shrink the widest columns first, leaving short columns alone.
    let mut total: usize = sizes.iter().sum();
    while total > available {
        let col = sizes.iter().enumerate().max_by_key(|(_, size)| **size).unwrap().0;
        sizes[col] -= 1;
        total -= 1;
    }
    let mut result = vec![];
    for (row, cells) in rows.into_iter().enumerate() {
        let base = Style { bold: row == 0, ..Style::new(if row == 0 { ACCENT } else { Color::Reset }) };
        let cells: Vec<_> = cells.into_iter().zip(&sizes).map(|(spans, &size)| wrap_spans(spans, size)).collect();
        for part in 0..cells.iter().map(Vec::len).max().unwrap_or(1) {
            let mut line = Line { spans: vec![] };
            for (col, &size) in sizes.iter().enumerate() {
                if col > 0 {
                    line.push(" │ ", Style::new(GRAY));
                }
                let cell = cells[col].get(part);
                let used = cell.map_or(0, |line| line.spans.iter().map(|span| span.text.width()).sum());
                let padding = size - used;
                let left = match table.alignment[col] {
                    Alignment::None | Alignment::Left => 0,
                    Alignment::Center => padding / 2,
                    Alignment::Right => padding,
                };
                line.push(&" ".repeat(left), base);
                if let Some(cell) = cell {
                    for span in &cell.spans {
                        line.push(&span.text, span.style);
                    }
                }
                line.push(&" ".repeat(padding - left), base);
            }
            result.push(line);
        }
        if row == 0 {
            result.push(Line::new(sizes.iter().map(|&size| "─".repeat(size)).collect::<Vec<_>>().join("─┼─"), GRAY));
        }
    }
    Some(result)
}

/// Render sanitized, source-visible Markdown within `width` columns, without UI indentation.
pub(crate) fn render(text: &str, width: usize) -> Vec<Line> {
    let text = clean(text);
    let (source, tables) = parse(&text);
    let mut output = Line { spans: vec![] };
    let mut cursor = 0;
    for table in tables {
        let Some(lines) = table_lines(&source, &table, width) else { continue };
        // Replace the whole source line, including optional table indentation.
        let start = text[..table.range.start].rfind('\n').map_or(0, |i| i + 1);
        output.spans.extend(source.spans(cursor..start));
        for (i, line) in lines.into_iter().enumerate() {
            if i > 0 {
                output.push("\n", Style::new(Color::Reset));
            }
            output.spans.extend(line.spans);
        }
        if text[table.range.clone()].ends_with('\n') {
            output.push("\n", Style::new(Color::Reset));
        }
        cursor = table.range.end;
    }
    output.spans.extend(source.spans(cursor..text.len()));
    wrap_spans(output.spans, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.spans.iter().map(|span| span.text.as_str()).collect()).collect()
    }

    fn styled_text(lines: &[Line], include: impl Fn(Style) -> bool) -> String {
        lines
            .iter()
            .flat_map(|line| &line.spans)
            .filter(|span| include(span.style))
            .map(|span| span.text.as_str())
            .collect()
    }

    #[test]
    fn tables_align_cells_and_handle_escaped_pipes_and_uneven_rows() {
        let source = "Name | Qty | Status\n\
                      | :--- | ---: | :---: |\n\
                      | a\\|b | 7 | x |\n\
                      `p\\|q` | 42 | ok\n\
                      short | 9\n\
                      extra | 100 | yes | ignored\n\n\
                      after";
        let lines = render(source, 80);
        assert_eq!(
            plain(&lines),
            [
                "Name  │ Qty │ Status",
                "──────┼─────┼───────",
                "a|b   │   7 │   x   ",
                "`p|q` │  42 │   ok  ",
                "short │   9 │       ",
                "extra │ 100 │  yes  ",
                "",
                "after",
            ]
        );
        assert!(
            lines[0]
                .spans
                .iter()
                .filter(|span| span.style.color != GRAY)
                .all(|span| { span.style.color == ACCENT && span.style.bold })
        );
        assert_eq!(styled_text(&lines, |style| style.color == Color::Yellow), "`p|q`");
    }

    #[test]
    fn wrapped_tables_preserve_cell_content_and_stay_within_unicode_width() {
        let source = "A | B\n--- | ---\n茶🍵⚠️👩‍💻 | abcdefgh";
        for width in 7..=24 {
            let rows = plain(&render(source, width));
            assert!(rows.iter().all(|row| row.width() <= width), "width {width}: {rows:?}");
            let cells: Vec<_> = rows.iter().skip(2).map(|row| row.split_once(" │ ").unwrap()).collect();
            assert_eq!(cells.iter().map(|(left, _)| left.trim()).collect::<String>(), "茶🍵⚠️👩‍💻");
            assert_eq!(cells.iter().map(|(_, right)| right.trim()).collect::<String>(), "abcdefgh");
            let column_width = rows[0].split_once(" │ ").unwrap().0.width();
            assert!(cells.iter().all(|(left, _)| left.width() == column_width), "width {width}: {rows:?}");
        }
        assert_eq!(plain(&render(source, 7)), ["A  │ B ", "───┼───", "茶 │ ab", "🍵 │ cd", "⚠️ │ ef", "👩‍💻 │ gh"]);
    }

    #[test]
    fn too_narrow_tables_stay_literal() {
        let source = "| A | B |\n| --- | --- |\n| x | y |";
        for width in 0..7 {
            let rows = plain(&render(source, width));
            assert_eq!(rows.concat(), source.replace('\n', ""));
            assert!(rows.iter().all(|row| row.width() <= width.max(1)));
        }
    }

    #[test]
    fn inline_styles_survive_soft_wraps_without_styling_identifiers_or_escapes() {
        let source = r"**abcdef** _xy_ `z` [go](u) foo_bar_baz \*literal*";
        let lines = render(source, 4);
        let rows = plain(&lines);
        assert_eq!(rows.concat(), source);
        assert!(rows.iter().all(|row| row.width() <= 4));
        assert_eq!(styled_text(&lines, |style| style.bold), "**abcdef**");
        assert_eq!(styled_text(&lines, |style| style.italic), "_xy_");
        assert_eq!(styled_text(&lines, |style| style.color == Color::Yellow), "`z`");
        assert_eq!(styled_text(&lines, |style| style.underlined), "[go](u)");
    }

    #[test]
    fn standalone_rendering_strips_terminal_controls_and_expands_tabs() {
        let lines = render("\x1b[31m# Safe\x1b[0m\n\x1b]0;bad\x07hello\tworld\r\0", 80);
        assert_eq!(plain(&lines), ["# Safe", "hello    world"]);
        assert!(lines.iter().flat_map(|line| &line.spans).all(|span| { span.text.chars().all(|ch| !ch.is_control()) }));
    }
}
