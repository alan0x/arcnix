//! Terminal color and text attributes in captured program output.
//!
//! Select Graphic Rendition sequences become styled spans. Every other escape or
//! control sequence is removed, because a static log view has no cursor to move.
use glory::style::{Color, Style};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TermColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pen {
    pub foreground: TermColor,
    pub background: TermColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub strike: bool,
    pub conceal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub text: String,
    pub pen: Pen,
}

/// Keeps the pen between lines, as a terminal does.
#[derive(Default)]
pub(crate) struct Parser {
    pen: Pen,
}

impl Parser {
    /// Append one line, without its newline, to `out`.
    ///
    /// A carriage return starts the line again, so a progress bar redrawn in
    /// place shows only its final state. Attributes set before it still apply.
    pub fn line(&mut self, line: &str, out: &mut Vec<Span>) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let start = out.len();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => match chars.next() {
                    Some('[') => self.control(&mut chars),
                    Some(']' | 'P' | 'X' | '^' | '_') => skip_string(&mut chars),
                    Some(' '..='/') => {
                        while chars.next_if(|c| matches!(c, ' '..='/')).is_some() {}
                        chars.next();
                    }
                    _ => {}
                },
                '\u{9b}' => self.control(&mut chars),
                '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => skip_string(&mut chars),
                '\r' => out.truncate(start),
                '\t' => push(out, start, c, self.pen),
                c if c.is_control() => {}
                c => push(out, start, c, self.pen),
            }
        }
    }

    /// A control sequence after its introducer; only SGR (`m`) has an effect.
    fn control(&mut self, chars: &mut std::iter::Peekable<std::str::Chars>) {
        let mut parameters = String::new();
        while let Some(c) = chars.next_if(|c| matches!(c, '0'..='?')) {
            parameters.push(c);
        }
        let mut intermediate = false;
        while chars.next_if(|c| matches!(c, ' '..='/')).is_some() {
            intermediate = true;
        }
        let Some(last) = chars.next_if(|c| matches!(c, '@'..='~')) else {
            return;
        };
        // Private markers are only valid first, and SGR has none; xterm ignores such sequences.
        if last == 'm' && !intermediate && !parameters.contains(['<', '=', '>', '?']) {
            self.select(&parameters);
        }
    }

    fn select(&mut self, parameters: &str) {
        let groups: Vec<Vec<Option<u32>>> = parameters
            .split(';')
            // Empty means the default; an oversized value is kept as unknown, not read as 0.
            .map(|group| {
                group
                    .split(':')
                    .map(|v| (!v.is_empty()).then(|| v.parse().unwrap_or(u32::MAX)))
                    .collect()
            })
            .collect();
        let mut i = 0;
        while i < groups.len() {
            let group = &groups[i];
            i += 1;
            let code = group[0].unwrap_or(0);
            let pen = &mut self.pen;
            match code {
                0 => *pen = Pen::default(),
                1 => pen.bold = true,
                2 => pen.dim = true,
                3 => pen.italic = true,
                4 => pen.underline = group.get(1).is_none_or(|style| *style != Some(0)),
                7 => pen.inverse = true,
                8 => pen.conceal = true,
                9 => pen.strike = true,
                21 => pen.underline = true,
                22 => (pen.bold, pen.dim) = (false, false),
                23 => pen.italic = false,
                24 => pen.underline = false,
                27 => pen.inverse = false,
                28 => pen.conceal = false,
                29 => pen.strike = false,
                30..=37 => pen.foreground = TermColor::Indexed((code - 30) as u8),
                39 => pen.foreground = TermColor::Default,
                40..=47 => pen.background = TermColor::Indexed((code - 40) as u8),
                49 => pen.background = TermColor::Default,
                90..=97 => pen.foreground = TermColor::Indexed((code - 82) as u8),
                100..=107 => pen.background = TermColor::Indexed((code - 92) as u8),
                38 | 48 | 58 => {
                    let color = if group.len() > 1 {
                        extended(&group[1..])
                    } else {
                        let (color, used) = extended_list(&groups[i..]);
                        i += used;
                        color
                    };
                    match (code, color) {
                        (38, Some(color)) => pen.foreground = color,
                        (48, Some(color)) => pen.background = color,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

/// `5:n` or `2:r:g:b`, with an optional color space id before r (`2:id:r:g:b`).
fn extended(values: &[Option<u32>]) -> Option<TermColor> {
    match values {
        [Some(5), n] => index(*n),
        [Some(2), .., r, g, b] if values.len() <= 5 => rgb(*r, *g, *b),
        _ => None,
    }
}

/// `5;n` or `2;r;g;b` as separate parameters; returns how many it consumed.
fn extended_list(groups: &[Vec<Option<u32>>]) -> (Option<TermColor>, usize) {
    let value = |i: usize| groups.get(i).map(|group| group[0]);
    match value(0) {
        Some(Some(5)) => (value(1).and_then(index), 2.min(groups.len())),
        Some(Some(2)) => match (value(1), value(2), value(3)) {
            (Some(r), Some(g), Some(b)) => (rgb(r, g, b), 4),
            _ => (None, groups.len()),
        },
        _ => (None, 0),
    }
}

fn index(n: Option<u32>) -> Option<TermColor> {
    u8::try_from(n.unwrap_or(0)).ok().map(TermColor::Indexed)
}

fn rgb(r: Option<u32>, g: Option<u32>, b: Option<u32>) -> Option<TermColor> {
    let part = |v: Option<u32>| u8::try_from(v.unwrap_or(0)).ok();
    Some(TermColor::Rgb(part(r)?, part(g)?, part(b)?))
}

/// Skip an OSC, DCS or similar string through BEL or ST. Any other escape also
/// ends the string, as in a terminal, and is left to start the next sequence.
fn skip_string(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&c) = chars.peek() {
        match c {
            '\x07' | '\u{9c}' => {
                chars.next();
                return;
            }
            '\x1b' => {
                let mut ahead = chars.clone();
                ahead.next();
                if ahead.peek() == Some(&'\\') {
                    chars.next();
                    chars.next();
                }
                return;
            }
            _ => {
                chars.next();
            }
        }
    }
}

fn push(out: &mut Vec<Span>, start: usize, c: char, pen: Pen) {
    let in_line = out.len() > start;
    match out.last_mut() {
        Some(last) if in_line && last.pen == pen => last.text.push(c),
        _ => out.push(Span {
            text: c.to_string(),
            pen,
        }),
    }
}

/// The 256-color palette above the 16 themed colors.
fn palette(n: u8) -> Color {
    match n {
        16..=231 => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let n = n - 16;
            Color(level(n / 36), level(n / 6 % 6), level(n % 6), 255)
        }
        232.. => {
            let v = 8 + (n - 232) * 10;
            Color(v, v, v, 255)
        }
        _ => unreachable!("themed colors use classes"),
    }
}

impl Pen {
    /// Classes for the themed colors and attributes, plus a style for colors
    /// outside the 16-color theme palette.
    pub fn presentation(&self) -> (String, Style) {
        let mut foreground = self.foreground;
        if let TermColor::Indexed(n @ 0..=7) = foreground
            && self.bold
        {
            // Matches the terminal, which draws bold text in bright colors.
            foreground = TermColor::Indexed(n + 8);
        }
        let mut background = self.background;
        let mut classes = vec!["ansi"];
        if self.inverse {
            std::mem::swap(&mut foreground, &mut background);
            if foreground == TermColor::Default {
                classes.push("ansi-fg-inverse");
            }
            if background == TermColor::Default {
                classes.push("ansi-bg-inverse");
            }
        }
        let mut style = Style::new();
        let mut names = Vec::new();
        for (color, prefix, slot) in [
            (foreground, "ansi-fg-", &mut style.color),
            (background, "ansi-bg-", &mut style.background),
        ] {
            match color {
                TermColor::Default => {}
                TermColor::Indexed(n @ 0..=15) => names.push(format!("{prefix}{n}")),
                TermColor::Indexed(n) => *slot = Some(palette(n)),
                TermColor::Rgb(r, g, b) => *slot = Some(Color(r, g, b, 255)),
            }
        }
        for (on, class) in [
            (self.bold, "ansi-bold"),
            (self.dim, "ansi-dim"),
            (self.italic, "ansi-italic"),
            (self.underline, "ansi-underline"),
            (self.strike, "ansi-strike"),
            (self.conceal, "ansi-conceal"),
        ] {
            if on {
                classes.push(class);
            }
        }
        let mut class = classes.join(" ");
        for name in names {
            class.push(' ');
            class.push_str(&name);
        }
        (class, style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lines: &[&str]) -> Vec<Span> {
        let mut parser = Parser::default();
        let mut out = Vec::new();
        for line in lines {
            parser.line(line, &mut out);
        }
        out
    }
    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }
    fn fg(n: u8) -> Pen {
        Pen {
            foreground: TermColor::Indexed(n),
            ..Pen::default()
        }
    }

    #[test]
    fn plain_text_is_one_default_span() {
        assert_eq!(
            parse(&["plain\ttext"]),
            [Span {
                text: "plain\ttext".into(),
                pen: Pen::default()
            }]
        );
    }

    #[test]
    fn colors_and_resets_split_spans() {
        let spans = parse(&["\x1b[32mINFO\x1b[0m ready \x1b[1;91mfailed\x1b[m"]);
        assert_eq!(text(&spans), "INFO ready failed");
        assert_eq!(spans[0].pen, fg(2));
        assert_eq!(spans[1].pen, Pen::default());
        assert_eq!(
            spans[2].pen,
            Pen {
                bold: true,
                ..fg(9)
            }
        );
    }

    #[test]
    fn attributes_turn_on_and_off_independently() {
        let spans = parse(&["\x1b[1;2;3;4;7;9ma\x1b[22;23;24;27;29mb"]);
        let all = Pen {
            bold: true,
            dim: true,
            italic: true,
            underline: true,
            inverse: true,
            strike: true,
            ..Pen::default()
        };
        assert_eq!(spans[0].pen, all);
        assert_eq!(spans[1].pen, Pen::default());
        let hidden = parse(&["\x1b[8msecret\x1b[28mshown"]);
        assert!(hidden[0].pen.conceal && !hidden[1].pen.conceal);
        assert_eq!(hidden[0].pen.presentation().0, "ansi ansi-conceal");
        assert!(!parse(&["\x1b[4:0mx"])[0].pen.underline);
        assert!(parse(&["\x1b[4:3mx"])[0].pen.underline);
    }

    #[test]
    fn extended_colors_in_both_parameter_forms() {
        for (sequence, foreground, background) in [
            (
                "38;5;208;48;5;17",
                TermColor::Indexed(208),
                TermColor::Indexed(17),
            ),
            (
                "38;2;1;2;3;48;2;4;5;6",
                TermColor::Rgb(1, 2, 3),
                TermColor::Rgb(4, 5, 6),
            ),
            (
                "38:5:208;48:2::4:5:6",
                TermColor::Indexed(208),
                TermColor::Rgb(4, 5, 6),
            ),
            (
                "38:2:1:2:3;48:2:0:4:5:6",
                TermColor::Rgb(1, 2, 3),
                TermColor::Rgb(4, 5, 6),
            ),
        ] {
            let pen = parse(&[&format!("\x1b[{sequence}mx")])[0].pen;
            assert_eq!(
                (pen.foreground, pen.background),
                (foreground, background),
                "{sequence}"
            );
        }
        // The underline color's arguments are not read as further attributes.
        assert_eq!(parse(&["\x1b[58;5;1;32mx"])[0].pen, fg(2));
        assert_eq!(parse(&["\x1b[58;2;1;2;3;32mx"])[0].pen, fg(2));
        // Oversized codes and misplaced private markers are ignored, not read as a reset.
        assert_eq!(
            parse(&["\x1b[1;31;99999999999mx"])[0].pen,
            Pen {
                bold: true,
                ..fg(1)
            }
        );
        assert_eq!(parse(&["\x1b[31m\x1b[0;3?mx"])[0].pen, fg(1));
        // Out-of-range or missing values leave the color unchanged.
        assert_eq!(parse(&["\x1b[31;38;5;300mx"])[0].pen, fg(1));
        assert_eq!(parse(&["\x1b[31;38;2;1mx"])[0].pen, fg(1));
    }

    #[test]
    fn the_pen_carries_across_lines() {
        let spans = parse(&["\x1b[33mfirst", "second\x1b[0m", "third"]);
        assert_eq!(spans[0].pen, fg(3));
        assert_eq!(spans[1].pen, fg(3));
        assert_eq!(spans[1].text, "second");
        assert_eq!(spans[2].pen, Pen::default());
    }

    #[test]
    fn other_escapes_and_controls_are_removed() {
        let line = "\x1b[2K\x1b[1G\x1b[?25lA\x1b]0;title\x07B\x1b]8;;https://example.com\x1b\\C\x1b]8;;\x1b\\\x1b(BD\x1b=E\x07\x08F\x1b[>4;1mG\u{9b}31mH";
        let spans = parse(&[line]);
        assert_eq!(text(&spans), "ABCDEFGH");
        assert_eq!(spans.last().unwrap().pen, fg(1));
        // A new escape ends an unterminated string and still takes effect.
        let spans = parse(&["\x1b]0;title\x1b[32mServer started"]);
        assert_eq!(text(&spans), "Server started");
        assert_eq!(spans[0].pen, fg(2));
        // An unterminated sequence at the end of the line is dropped.
        assert_eq!(text(&parse(&["ok\x1b[31"])), "ok");
        assert_eq!(text(&parse(&["ok\x1b]0;title"])), "ok");
    }

    #[test]
    fn carriage_returns_keep_the_final_redraw() {
        assert_eq!(text(&parse(&["windows line\r"])), "windows line");
        let spans = parse(&["before", "\x1b[32m10%\r50%\r\x1b[1m100%"]);
        assert_eq!(text(&spans), "before100%");
        assert_eq!(
            spans[1].pen,
            Pen {
                bold: true,
                ..fg(2)
            }
        );
    }

    #[test]
    fn presentation_uses_theme_classes_for_the_16_colors() {
        let (class, style) = fg(1).presentation();
        assert_eq!(class, "ansi ansi-fg-1");
        assert_eq!(style, Style::new());
        let bold = Pen {
            bold: true,
            ..fg(4)
        };
        assert_eq!(bold.presentation().0, "ansi ansi-bold ansi-fg-12");
        let inverse = Pen {
            inverse: true,
            ..fg(2)
        };
        assert_eq!(inverse.presentation().0, "ansi ansi-fg-inverse ansi-bg-2");
        let plain_inverse = Pen {
            inverse: true,
            ..Pen::default()
        };
        assert_eq!(
            plain_inverse.presentation().0,
            "ansi ansi-fg-inverse ansi-bg-inverse"
        );
    }

    #[test]
    fn presentation_uses_inline_colors_beyond_the_16_colors() {
        let pen = Pen {
            foreground: TermColor::Indexed(196),
            background: TermColor::Rgb(1, 2, 3),
            ..Pen::default()
        };
        let (class, style) = pen.presentation();
        assert_eq!(class, "ansi");
        assert_eq!(style.color, Some(Color(255, 0, 0, 255)));
        assert_eq!(style.background, Some(Color(1, 2, 3, 255)));
        assert_eq!(palette(16), Color(0, 0, 0, 255));
        assert_eq!(palette(231), Color(255, 255, 255, 255));
        assert_eq!(palette(244), Color(128, 128, 128, 255));
    }
}
