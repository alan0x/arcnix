//! Captured log text with its terminal colors, rendered as inline spans.
use glory::prelude::*;
use glory::style::Style;

use crate::ansi::{Parser, Pen, Span};

/// One styled run of log text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LogSpan {
    pub text: String,
    pub class: String,
    pub style: Style,
}

/// Bounds the rendered nodes; the oldest spans are merged into plain text first.
pub(crate) const SPAN_LIMIT: usize = 4000;

#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// Text the program did not style, with an optional class such as a muted time.
    Plain(&'static str),
    Pen(Pen),
}

/// Collects log lines as spans. Adjacent text with the same look shares a span,
/// so uncolored logs stay a handful of nodes.
#[derive(Default)]
pub(crate) struct Spans {
    // Styles are computed only for the spans that are kept, after the limit.
    spans: Vec<(String, Look)>,
    parser: Parser,
    line: Vec<Span>,
}

impl Spans {
    /// Text that Arcnix adds, such as a timestamp, with an optional class.
    pub fn plain(&mut self, text: &str, class: &'static str) {
        self.push(text, Look::Plain(class));
    }
    /// One line of program output, without its newline. Colors carry over to
    /// the next line, as in a terminal, until [`Spans::reset`].
    pub fn output(&mut self, line: &str) {
        self.parser.line(line, &mut self.line);
        for span in std::mem::take(&mut self.line) {
            let look = if span.pen == Pen::default() {
                Look::Plain("")
            } else {
                Look::Pen(span.pen)
            };
            self.push(&span.text, look);
        }
    }
    /// Program output that may span several lines.
    pub fn output_lines(&mut self, text: &str) {
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.newline();
            }
            self.output(line);
        }
    }
    /// Starts the next output with default colors and attributes.
    pub fn reset(&mut self) {
        self.parser = Parser::default();
    }
    pub fn newline(&mut self) {
        match self.spans.last_mut() {
            Some((text, _)) if !text.ends_with('\n') => text.push('\n'),
            _ => self.push("\n", Look::Plain("")),
        }
    }
    /// A position to return to with [`Spans::truncate`] or read from with
    /// [`Spans::text_since`].
    pub fn mark(&self) -> (usize, usize) {
        let text = self.spans.last().map_or(0, |(text, _)| text.len());
        (self.spans.len(), text)
    }
    pub fn text_since(&self, (index, offset): (usize, usize)) -> String {
        let mut text = String::new();
        if let Some((first, _)) = index.checked_sub(1).and_then(|i| self.spans.get(i)) {
            text.push_str(&first[offset..]);
        }
        for (part, _) in self.spans.iter().skip(index) {
            text.push_str(part);
        }
        text
    }
    /// Removes the text after `mark`. Colors set by that text still apply
    /// until [`Spans::reset`].
    pub fn truncate(&mut self, (index, offset): (usize, usize)) {
        self.spans.truncate(index);
        if let Some((text, _)) = self.spans.last_mut() {
            text.truncate(offset);
        }
    }
    /// True when there is nothing to show but whitespace.
    pub fn is_blank(&self) -> bool {
        self.spans.iter().all(|(text, _)| text.trim().is_empty())
    }
    pub fn finish(mut self) -> Vec<LogSpan> {
        if self.spans.len() > SPAN_LIMIT {
            let excess = self.spans.len() - SPAN_LIMIT + 1;
            let merged = self.spans.drain(..excess).map(|(text, _)| text).collect();
            self.spans.insert(0, (merged, Look::Plain("")));
        }
        self.spans
            .into_iter()
            .map(|(text, look)| {
                let (class, style) = match look {
                    Look::Plain(class) => (class.into(), Style::new()),
                    Look::Pen(pen) => pen.presentation(),
                };
                LogSpan { text, class, style }
            })
            .collect()
    }
    fn push(&mut self, text: &str, look: Look) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some((last, last_look)) if *last_look == look => last.push_str(text),
            _ => self.spans.push((text.into(), look)),
        }
    }
}

/// The spans as selectable inline text in one preformatted block.
pub(crate) fn view_spans(class: &'static str, spans: &[LogSpan]) -> impl Widget + use<> {
    view().class(class).fill(
        spans
            .iter()
            .map(|span| {
                text()
                    .kind(TextKind::Inline)
                    .selectable(true)
                    .class(span.class.clone())
                    .style(span.style.clone())
                    .content(span.text.clone())
            })
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(spans: &[LogSpan]) -> Vec<(&str, &str)> {
        spans
            .iter()
            .map(|s| (s.text.as_str(), s.class.as_str()))
            .collect()
    }

    #[test]
    fn uncolored_text_shares_one_span() {
        let mut spans = Spans::default();
        spans.plain("09:00 ", "");
        spans.output("ready");
        spans.newline();
        spans.output("\x1b[0mdone\x1b[m");
        spans.newline();
        assert_eq!(parts(&spans.finish()), [("09:00 ready\ndone\n", "")]);
    }

    #[test]
    fn reset_stops_colors_from_carrying_over() {
        let mut spans = Spans::default();
        spans.output("\x1b[31mred");
        spans.newline();
        spans.output("still red");
        spans.newline();
        spans.reset();
        spans.output("plain");
        assert_eq!(
            parts(&spans.finish()),
            [("red\nstill red\n", "ansi ansi-fg-1"), ("plain", "")]
        );
    }

    #[test]
    fn marks_read_and_remove_the_text_after_them() {
        let mut spans = Spans::default();
        spans.plain("kept ", "");
        let mark = spans.mark();
        spans.output_lines("a\x1b[32mb\nc");
        assert_eq!(spans.text_since(mark), "ab\nc");
        spans.truncate(mark);
        spans.reset();
        spans.output("d");
        assert_eq!(parts(&spans.finish()), [("kept d", "")]);
    }

    #[test]
    fn spans_are_bounded_keeping_the_newest_colors() {
        let mut spans = Spans::default();
        for _ in 0..SPAN_LIMIT {
            spans.output("\x1b[31mr\x1b[32mg");
            spans.newline();
        }
        let spans = spans.finish();
        assert_eq!(spans.len(), SPAN_LIMIT);
        assert_eq!(spans[0].class, "");
        assert_eq!(spans.last().unwrap().class, "ansi ansi-fg-2");
        let text: String = spans.into_iter().map(|s| s.text).collect();
        assert_eq!(text, "rg\n".repeat(SPAN_LIMIT));
    }
}
