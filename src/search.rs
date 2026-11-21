//! Searching the screen and its scrollback.

use unicode_width::UnicodeWidthChar;

use crate::palette::Palette;
use crate::render::{CellFlags, CellView, Color, Cursor, CursorShape, Frame};
use crate::selection::{self, Point, Range};
use crate::vt::{Flags, Term};

/// An open search: what was typed and where it was found.
///
/// All matches are found again when the query changes or the user moves to
/// another match. Output only updates them ([`Search::follow`]): rescanning
/// a long scrollback for each chunk a busy program prints would be slow.
#[derive(Debug, Default)]
pub struct Search {
    query: String,
    /// In reading order.
    matches: Vec<Range>,
    current: Option<usize>,
    /// The scrollback when `matches` were last brought up to date.
    scanned: Option<Scanned>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scanned {
    alt_screen: bool,
    dropped: u64,
    scrollback_len: usize,
}

impl Scanned {
    fn of(term: &Term) -> Self {
        Self {
            alt_screen: term.is_alt_screen(),
            dropped: term.scrollback_dropped(),
            scrollback_len: term.grid().scrollback_len(),
        }
    }
}

impl Search {
    pub fn push(&mut self, text: &str, term: &Term) {
        self.query.push_str(text);
        self.refresh(term);
    }

    pub fn pop(&mut self, term: &Term) {
        self.query.pop();
        self.refresh(term);
    }

    /// Moves to the next match up (older output), or down with `down`,
    /// wrapping around at either end.
    pub fn step(&mut self, term: &Term, down: bool) {
        self.refresh(term);
        let n = self.matches.len();
        self.current = self
            .current
            .map(|i| if down { (i + 1) % n } else { (i + n - 1) % n });
    }

    pub fn current(&self) -> Option<Range> {
        self.matches.get(self.current?).copied()
    }

    /// Whether `p` is in a match, and if so whether that is the current one.
    pub fn hit(&self, p: Point) -> Option<bool> {
        let i = self
            .matches
            .partition_point(|r| r.start <= p)
            .checked_sub(1)?;
        self.matches[i]
            .contains(p)
            .then_some(self.current == Some(i))
    }

    /// Finds the matches again and stays on the current one if it is still
    /// there; the first search starts from the bottom, the newest output.
    pub fn refresh(&mut self, term: &Term) {
        let at = self.current();
        self.matches = find(term, &self.query, 0);
        self.scanned = Some(Scanned::of(term));
        self.stay_at(at);
    }

    /// Brings the matches up to date after output. Scrollback rows do not
    /// change, so their matches are kept and moved up by the rows dropped
    /// above them; only the rows that were on the screen are searched again.
    pub fn follow(&mut self, term: &Term) {
        let now = Scanned::of(term);
        let Some(before) = self.scanned.replace(now) else {
            return;
        };
        let shift = now
            .dropped
            .checked_sub(before.dropped)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|_| before.alt_screen == now.alt_screen);
        let Some(shift) = shift else {
            // The other screen's scrollback: its rows are unrelated.
            return self.refresh(term);
        };
        let at = self.current().and_then(|r| {
            Some(Range {
                start: Point::new(r.start.line.checked_sub(shift)?, r.start.col),
                end: Point::new(r.end.line.checked_sub(shift)?, r.end.col),
            })
        });
        let kept = before
            .scrollback_len
            .saturating_sub(shift)
            .min(now.scrollback_len);
        self.matches
            .retain(|m| m.start.line >= shift && m.start.line < kept + shift);
        for m in &mut self.matches {
            m.start.line -= shift;
            m.end.line -= shift;
        }
        self.matches.extend(find(term, &self.query, kept));
        match at {
            Some(_) => self.stay_at(at),
            // The current match scrolled out of the scrollback: the oldest
            // one left is the nearest.
            None => self.current = (!self.matches.is_empty()).then_some(0),
        }
    }

    /// Makes the current match `at`, or the nearest one above it; with no
    /// `at`, the newest.
    fn stay_at(&mut self, at: Option<Range>) {
        let before = at.map_or(self.matches.len(), |r| {
            self.matches.partition_point(|m| m.start <= r.start)
        });
        self.current = before
            .checked_sub(1)
            .or_else(|| self.matches.len().checked_sub(1));
    }
}

/// Case-insensitive matches of `query` within single rows, from absolute
/// line `from` down.
pub fn find(term: &Term, query: &str, from: usize) -> Vec<Range> {
    let query: Vec<char> = query.chars().map(fold).collect();
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for line in from..selection::line_count(term) {
        let cells: Vec<(usize, char, bool)> = selection::row(term, line)
            .cells()
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.flags.contains(Flags::WIDE_SPACER))
            .map(|(col, c)| (col, fold(c.c), c.flags.contains(Flags::WIDE)))
            .collect();
        let mut i = 0;
        while i + query.len() <= cells.len() {
            if cells[i..i + query.len()]
                .iter()
                .zip(&query)
                .all(|(c, q)| c.1 == *q)
            {
                let last = cells[i + query.len() - 1];
                out.push(Range {
                    start: Point::new(line, cells[i].0),
                    end: Point::new(line, last.0 + usize::from(last.2)),
                });
                i += query.len();
            } else {
                i += 1;
            }
        }
    }
    out
}

/// One char per cell to compare: the first char of the lowercase form, so
/// that a char whose lowercase is longer ('İ') still lines up with its cell.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The display offset that puts absolute `line` in the middle of the view,
/// or `None` when it is already in view.
pub fn offset_to_show(term: &Term, line: usize) -> Option<usize> {
    let top = selection::absolute_line(term, 0);
    if (top..top + term.rows()).contains(&line) {
        return None;
    }
    Some((term.grid().scrollback_len() + term.rows() / 2).saturating_sub(line))
}

/// Draws the search bar over the bottom row and puts the cursor after the
/// query. Returns the cursor column, where an IME composes.
pub fn bar(frame: &mut Frame, search: &Search, palette: &Palette) -> u16 {
    let row = frame.rows.saturating_sub(1);
    let (fg, bg) = (palette.background, palette.foreground);
    for col in 0..frame.cols {
        if let Some(cell) = frame.cell_mut(col, row) {
            *cell = CellView::blank(fg, bg);
        }
    }
    let count = match (search.current, search.matches.len()) {
        (_, 0) if !search.query.is_empty() => "見つかりません ".to_owned(),
        (Some(i), n) => format!("{}/{n} ", i + 1),
        _ => String::new(),
    };
    let count_width = width(&count);
    let right = frame.cols.saturating_sub(count_width);
    put(frame, right, row, &count, (fg, bg), frame.cols);
    let caret = put(
        frame,
        0,
        row,
        &format!(" 検索: {}", search.query),
        (fg, bg),
        right,
    );
    frame.cursor = (caret < frame.cols).then_some(Cursor {
        col: caret,
        row,
        shape: CursorShape::Bar,
        color: fg,
    });
    caret
}

fn width(s: &str) -> u16 {
    s.chars().map(|c| c.width().unwrap_or(0) as u16).sum()
}

/// Writes `text` from column `x` up to `limit`; returns the column after it.
fn put(
    frame: &mut Frame,
    mut x: u16,
    row: u16,
    text: &str,
    (fg, bg): (Color, Color),
    limit: u16,
) -> u16 {
    for c in text.chars() {
        let w = c.width().unwrap_or(0).min(2) as u16;
        if w == 0 || x + w > limit {
            continue;
        }
        if let Some(cell) = frame.cell_mut(x, row) {
            cell.ch = c;
            cell.flags = if w == 2 {
                CellFlags::WIDE
            } else {
                CellFlags::empty()
            };
        }
        if w == 2
            && let Some(cell) = frame.cell_mut(x + 1, row)
        {
            *cell = CellView {
                flags: CellFlags::WIDE_SPACER,
                ..CellView::blank(fg, bg)
            };
        }
        x += w;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize, input: &str) -> Term {
        let mut term = Term::new(cols, rows);
        term.advance(input.as_bytes());
        term
    }

    fn find0(t: &Term, query: &str) -> Vec<Range> {
        find(t, query, 0)
    }

    fn range(line: usize, start: usize, end: usize) -> Range {
        Range {
            start: Point::new(line, start),
            end: Point::new(line, end),
        }
    }

    #[test]
    fn matches_ignore_case_and_cover_wide_characters() {
        let t = term(20, 3, "Error error\r\nエラーerror\r\n");
        assert_eq!(
            find0(&t, "ERROR"),
            [range(0, 0, 4), range(0, 6, 10), range(1, 6, 10)]
        );
        assert_eq!(find0(&t, "ラー"), [range(1, 2, 5)]);
        assert!(find0(&t, "").is_empty());
    }

    #[test]
    fn scrollback_is_searched() {
        let t = term(10, 2, "needle\r\na\r\nb\r\nc");
        assert_eq!(selection::line_count(&t), 4);
        assert_eq!(find0(&t, "needle"), [range(0, 0, 5)]);
    }

    #[test]
    fn search_starts_at_the_newest_match_and_enter_goes_up() {
        let t = term(10, 3, "ab\r\nab\r\nab");
        let mut s = Search::default();
        s.push("a", &t);
        assert_eq!(s.current(), Some(range(2, 0, 0)));
        s.step(&t, false);
        assert_eq!(s.current(), Some(range(1, 0, 0)));
        s.step(&t, true);
        s.step(&t, true);
        assert_eq!(s.current(), Some(range(0, 0, 0)), "wraps to the top");
    }

    #[test]
    fn typing_more_stays_on_the_current_match_when_it_still_matches() {
        let t = term(10, 3, "abc\r\nabd\r\nabc");
        let mut s = Search::default();
        s.push("ab", &t);
        s.step(&t, false);
        assert_eq!(s.current(), Some(range(1, 0, 1)));
        s.push("c", &t);
        assert_eq!(s.current(), Some(range(0, 0, 2)), "nearest match above");
        s.pop(&t);
        s.push("d", &t);
        assert_eq!(s.current(), Some(range(1, 0, 2)));
    }

    #[test]
    fn output_below_keeps_the_matches_and_finds_new_ones() {
        let mut t = term(10, 2, "ab\r\nab");
        let mut s = Search::default();
        s.push("ab", &t);
        s.step(&t, false);
        assert_eq!(s.current(), Some(range(0, 0, 1)));
        t.advance(b"\r\nxx\r\nab");
        s.follow(&t);
        assert_eq!(s.matches, find0(&t, "ab"));
        assert_eq!(s.current(), Some(range(0, 0, 1)), "still on the same text");
    }

    #[test]
    fn matches_move_up_with_rows_dropped_from_a_full_scrollback() {
        let mut t = term(10, 2, "ab\r\ncd\r\nab\r\ncd");
        t.set_scrollback_limit(2);
        let mut s = Search::default();
        s.push("cd", &t);
        s.step(&t, false);
        assert_eq!(s.current(), Some(range(1, 0, 1)));
        t.advance(b"\r\nxx");
        s.follow(&t);
        assert_eq!(t.scrollback_dropped(), 1);
        assert_eq!(s.matches, find0(&t, "cd"));
        assert_eq!(s.current(), Some(range(0, 0, 1)), "still on the same text");
        t.advance(b"\r\nxx\r\nxx\r\nxx");
        s.follow(&t);
        assert!(s.matches.is_empty());
        assert_eq!(s.current(), None);
    }

    #[test]
    fn rewritten_screen_rows_are_searched_again() {
        let mut t = term(10, 2, "ab\r\nab");
        let mut s = Search::default();
        s.push("ab", &t);
        t.advance(b"\x1b[2J\x1b[Hxx ab");
        s.follow(&t);
        assert_eq!(s.matches, [range(0, 3, 4)]);
    }

    #[test]
    fn switching_screens_or_a_reset_leaves_no_stale_matches() {
        let mut t = term(10, 2, "ab\r\nab\r\nab");
        let mut s = Search::default();
        s.push("ab", &t);
        t.advance(b"\x1b[?1049h\x1b[Hab");
        s.follow(&t);
        assert_eq!(s.matches, find0(&t, "ab"));
        t.advance(b"\x1b[?1049l");
        s.follow(&t);
        assert_eq!(s.matches, find0(&t, "ab"));
        t.advance(b"\x1bc");
        s.follow(&t);
        assert_eq!(s.matches, find0(&t, "ab"));
        assert!(s.matches.is_empty());
    }

    #[test]
    fn hits_tell_the_current_match_from_the_others() {
        let t = term(10, 1, "ab ab");
        let mut s = Search::default();
        s.push("ab", &t);
        assert_eq!(s.hit(Point::new(0, 0)), Some(false));
        assert_eq!(s.hit(Point::new(0, 2)), None);
        assert_eq!(s.hit(Point::new(0, 4)), Some(true));
    }

    #[test]
    fn a_match_out_of_view_is_scrolled_to_the_middle() {
        let t = term(10, 4, &"x\r\n".repeat(20));
        assert_eq!(t.grid().scrollback_len(), 17);
        assert_eq!(offset_to_show(&t, 18), None);
        assert_eq!(offset_to_show(&t, 5), Some(14));
    }

    #[test]
    fn the_bar_shows_the_query_and_the_count() {
        let t = term(20, 2, "foo foo");
        let mut s = Search::default();
        s.push("foo", &t);
        let p = Palette::default();
        let mut frame = Frame::new(20, 2, p.foreground, p.background);
        let caret = bar(&mut frame, &s, &p);
        let text: String = (0..20)
            .filter_map(|c| frame.cell(c, 1))
            .filter(|c| !c.flags.contains(CellFlags::WIDE_SPACER))
            .map(|c| c.ch)
            .collect();
        assert_eq!(text.trim_end(), " 検索: foo      2/2");
        assert_eq!(caret, 10);
        assert_eq!(frame.cursor.unwrap().col, 10);
    }
}
