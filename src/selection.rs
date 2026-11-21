//! Text selection over the screen and its scrollback.
//!
//! Positions use absolute lines: line 0 is the oldest scrollback row and
//! the live screen follows it. A line keeps its number while output scrolls
//! it into the scrollback, so a selection stays on the same text.

use std::time::{Duration, Instant};

use crate::vt::{Flags, Row, Term};

/// Longest gap between clicks that still counts as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// Default characters that end a word for double-click selection, besides
/// whitespace. Path and URL punctuation (`/ . - _ ~ : ? & = # %`) is kept
/// inside words so a double-click picks up a whole path or URL.
pub const WORD_DELIMITERS: &str = ",│`|;\"'()[]{}<>";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: usize,
    pub col: usize,
}

impl Point {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// What a click selects, and what dragging extends by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Cell,
    Word,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    anchor: Point,
    head: Point,
    unit: Unit,
}

/// Inclusive range of cells, `start <= end`, in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Point,
    pub end: Point,
}

impl Range {
    pub fn contains(&self, p: Point) -> bool {
        self.start <= p && p <= self.end
    }
}

impl Selection {
    pub fn new(at: Point, unit: Unit) -> Self {
        Self {
            anchor: at,
            head: at,
            unit,
        }
    }

    /// Moves the end that is not the anchor, as dragging or Shift+click do.
    pub fn extend(&mut self, to: Point) {
        self.head = to;
    }

    /// The selected cells, grown to whole words or lines and to both halves
    /// of wide characters. `None` when a plain click selected nothing.
    /// Words end at whitespace and at the characters in `delimiters`.
    pub fn range(&self, term: &Term, delimiters: &str) -> Option<Range> {
        if self.unit == Unit::Cell && self.anchor == self.head {
            return None;
        }
        let (start, end) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let (start, end) = match self.unit {
            Unit::Cell => (start, end),
            Unit::Word => (
                word_start(term, start, delimiters),
                word_end(term, end, delimiters),
            ),
            Unit::Line => (line_start(term, start), line_end(term, end)),
        };
        Some(Range {
            start: wide_start(term, start),
            end: wide_end(term, end),
        })
    }
}

/// Turns successive clicks on the same cell into single, double and triple
/// clicks.
#[derive(Debug, Default)]
pub struct Clicks {
    last: Option<(Instant, Point, Unit)>,
}

impl Clicks {
    pub fn click(&mut self, at: Point, now: Instant) -> Unit {
        let unit = match self.last {
            Some((time, point, unit)) if point == at && now - time <= MULTI_CLICK => match unit {
                Unit::Cell => Unit::Word,
                Unit::Word => Unit::Line,
                Unit::Line => Unit::Cell,
            },
            _ => Unit::Cell,
        };
        self.last = Some((now, at, unit));
        unit
    }
}

/// Number of absolute lines: scrollback plus screen.
pub fn line_count(term: &Term) -> usize {
    term.grid().scrollback_len() + term.rows()
}

/// Absolute line of display line `line` at the current scroll position.
pub fn absolute_line(term: &Term, line: usize) -> usize {
    term.grid().scrollback_len() - term.display_offset() + line
}

pub fn clamp_u16(v: usize) -> u16 {
    u16::try_from(v).unwrap_or(u16::MAX)
}

pub fn row(term: &Term, line: usize) -> &Row {
    let scrollback = term.grid().scrollback_len();
    if line < scrollback {
        term.grid().scrollback_row(line)
    } else {
        term.grid().row(line - scrollback)
    }
}

/// The selected text. Rows joined by auto-wrap stay on one line; other
/// rows end with `\n` and lose their trailing blanks.
pub fn text(term: &Term, range: Range) -> String {
    let mut out = String::new();
    let last_line = range.end.line.min(line_count(term).saturating_sub(1));
    for line in range.start.line..=last_line {
        let row = row(term, line);
        let cells = row.cells();
        let from = if line == range.start.line {
            range.start.col
        } else {
            0
        };
        let to = if line == range.end.line {
            range.end.col.min(cells.len().saturating_sub(1))
        } else {
            cells.len().saturating_sub(1)
        };
        let begin = out.len();
        for cell in cells.get(from..=to).unwrap_or_default() {
            if cell.flags.contains(Flags::WIDE_SPACER) {
                continue;
            }
            out.push(cell.c);
            out.extend(cell.zerowidth());
        }
        let joined = row.wrapped() && to + 1 == cells.len();
        if !joined {
            let trimmed = begin + out[begin..].trim_end_matches(' ').len();
            out.truncate(trimmed);
            if line != last_line {
                out.push('\n');
            }
        }
    }
    out
}

fn is_word_char(c: char, delimiters: &str) -> bool {
    !c.is_whitespace() && !delimiters.contains(c)
}

fn cell_char(term: &Term, p: Point) -> char {
    row(term, p.line).cells().get(p.col).map_or(' ', |c| c.c)
}

/// The column to test for a word character: the right half of a wide
/// character belongs to the left half.
fn char_col(term: &Term, p: Point) -> usize {
    let cells = row(term, p.line).cells();
    match cells.get(p.col) {
        Some(c) if c.flags.contains(Flags::WIDE_SPACER) && p.col > 0 => p.col - 1,
        _ => p.col,
    }
}

fn word_start(term: &Term, p: Point, delimiters: &str) -> Point {
    let mut col = char_col(term, p);
    if !is_word_char(cell_char(term, Point::new(p.line, col)), delimiters) {
        return Point::new(p.line, col);
    }
    while col > 0 {
        let prev = char_col(term, Point::new(p.line, col - 1));
        if !is_word_char(cell_char(term, Point::new(p.line, prev)), delimiters) {
            break;
        }
        col = prev;
    }
    Point::new(p.line, col)
}

fn word_end(term: &Term, p: Point, delimiters: &str) -> Point {
    let cols = row(term, p.line).cells().len();
    let mut col = char_col(term, p);
    if !is_word_char(cell_char(term, Point::new(p.line, col)), delimiters) {
        return Point::new(p.line, col);
    }
    loop {
        let mut next = col + 1;
        if row(term, p.line)
            .cells()
            .get(next)
            .is_some_and(|c| c.flags.contains(Flags::WIDE_SPACER))
        {
            next += 1;
        }
        if next >= cols || !is_word_char(cell_char(term, Point::new(p.line, next)), delimiters) {
            break;
        }
        col = next;
    }
    Point::new(p.line, col)
}

/// Start of the logical line (rows joined by auto-wrap) containing `p`.
fn line_start(term: &Term, p: Point) -> Point {
    let mut line = p.line;
    while line > 0 && row(term, line - 1).wrapped() {
        line -= 1;
    }
    Point::new(line, 0)
}

fn line_end(term: &Term, p: Point) -> Point {
    let mut line = p.line;
    while line + 1 < line_count(term) && row(term, line).wrapped() {
        line += 1;
    }
    Point::new(line, row(term, line).cells().len().saturating_sub(1))
}

fn wide_start(term: &Term, p: Point) -> Point {
    Point::new(p.line, char_col(term, p))
}

fn wide_end(term: &Term, p: Point) -> Point {
    let wide = row(term, p.line)
        .cells()
        .get(p.col)
        .is_some_and(|c| c.flags.contains(Flags::WIDE));
    Point::new(p.line, if wide { p.col + 1 } else { p.col })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize, input: &str) -> Term {
        let mut term = Term::new(cols, rows);
        term.advance(input.as_bytes());
        term
    }

    fn selected(term: &Term, from: (usize, usize), to: (usize, usize), unit: Unit) -> String {
        let mut sel = Selection::new(Point::new(from.0, from.1), unit);
        sel.extend(Point::new(to.0, to.1));
        text(term, sel.range(term, WORD_DELIMITERS).unwrap())
    }

    #[test]
    fn quick_clicks_on_one_cell_cycle_through_cell_word_and_line() {
        let mut clicks = Clicks::default();
        let p = Point::new(0, 0);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(clicks.click(p, ms(0)), Unit::Cell);
        assert_eq!(clicks.click(p, ms(100)), Unit::Word);
        assert_eq!(clicks.click(p, ms(200)), Unit::Line);
        assert_eq!(clicks.click(p, ms(300)), Unit::Cell);
        // Too slow, or on another cell: a new single click.
        assert_eq!(clicks.click(p, ms(1000)), Unit::Cell);
        assert_eq!(clicks.click(Point::new(0, 1), ms(1100)), Unit::Cell);
    }

    #[test]
    fn a_click_without_dragging_selects_nothing() {
        let t = term(10, 2, "hello");
        assert_eq!(
            Selection::new(Point::new(0, 1), Unit::Cell).range(&t, WORD_DELIMITERS),
            None
        );
    }

    #[test]
    fn dragging_selects_the_cells_between_in_either_direction() {
        let t = term(10, 2, "hello");
        assert_eq!(selected(&t, (0, 1), (0, 3), Unit::Cell), "ell");
        assert_eq!(selected(&t, (0, 3), (0, 1), Unit::Cell), "ell");
    }

    #[test]
    fn rows_end_with_newlines_and_lose_trailing_blanks() {
        let t = term(10, 3, "ab\r\ncd\r\nef");
        assert_eq!(selected(&t, (0, 0), (2, 9), Unit::Cell), "ab\ncd\nef");
    }

    #[test]
    fn wrapped_rows_join_into_one_line() {
        let t = term(4, 3, "abcdefg\r\nh");
        assert_eq!(selected(&t, (0, 0), (2, 3), Unit::Cell), "abcdefg\nh");
    }

    #[test]
    fn double_click_selects_a_word_and_keeps_paths_whole() {
        let t = term(40, 1, "ls (/usr/local/bin) https://a.b/c?d=e");
        assert_eq!(selected(&t, (0, 8), (0, 8), Unit::Word), "/usr/local/bin");
        assert_eq!(
            selected(&t, (0, 25), (0, 25), Unit::Word),
            "https://a.b/c?d=e"
        );
        assert_eq!(selected(&t, (0, 0), (0, 0), Unit::Word), "ls");
    }

    #[test]
    fn word_delimiters_can_be_replaced() {
        let t = term(40, 1, "ls (/usr/local/bin) a,b");
        let word = |at: usize, delimiters: &str| {
            let sel = Selection::new(Point::new(0, at), Unit::Word);
            text(&t, sel.range(&t, delimiters).unwrap())
        };
        assert_eq!(word(10, "/"), "local");
        assert_eq!(word(21, ""), "a,b");
        assert_eq!(word(4, ""), "(/usr/local/bin)", "only whitespace is left");
    }

    #[test]
    fn double_click_on_a_blank_selects_just_that_cell() {
        let t = term(10, 1, "a  b");
        let mut sel = Selection::new(Point::new(0, 1), Unit::Word);
        sel.extend(Point::new(0, 1));
        let range = sel.range(&t, WORD_DELIMITERS).unwrap();
        assert_eq!((range.start.col, range.end.col), (1, 1));
    }

    #[test]
    fn dragging_after_a_double_click_extends_by_words() {
        let t = term(20, 1, "one two three");
        assert_eq!(selected(&t, (0, 1), (0, 9), Unit::Word), "one two three");
    }

    #[test]
    fn triple_click_selects_the_whole_wrapped_line() {
        let t = term(4, 3, "abcdefg\r\nh");
        assert_eq!(selected(&t, (1, 1), (1, 1), Unit::Line), "abcdefg");
    }

    #[test]
    fn wide_characters_are_selected_whole() {
        let t = term(10, 1, "a日本b");
        // Starting on the right half of 日 and ending on the left half of 本.
        let mut sel = Selection::new(Point::new(0, 2), Unit::Cell);
        sel.extend(Point::new(0, 3));
        let range = sel.range(&t, WORD_DELIMITERS).unwrap();
        assert_eq!((range.start.col, range.end.col), (1, 4));
        assert_eq!(text(&t, range), "日本");
        assert_eq!(selected(&t, (0, 3), (0, 3), Unit::Word), "a日本b");
    }

    #[test]
    fn lines_in_the_scrollback_keep_their_numbers() {
        let mut t = term(5, 2, "1\r\n2\r\n3");
        // "1" scrolled off: scrollback is ["1"], screen is ["2", "3"].
        assert_eq!(line_count(&t), 3);
        assert_eq!(absolute_line(&t, 0), 1);
        t.scroll_display(1);
        assert_eq!(absolute_line(&t, 0), 0);
        assert_eq!(selected(&t, (0, 0), (1, 4), Unit::Cell), "1\n2");
    }
}
