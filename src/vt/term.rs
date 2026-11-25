use std::collections::VecDeque;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vte::{Params, Parser};

use std::sync::Arc;

use crate::vt::cell::{Cell, Color, Flags, Hyperlink, MAX_ZEROWIDTH};
use crate::vt::grid::{Grid, Row};
use crate::vt::image::{Image, ImageStore, Placement};
use crate::vt::keyboard::{KeyboardFlags, KeyboardStack};
use crate::vt::kitty::ApcScanner;
use crate::vt::modes::{CursorStyle, Modes};
use crate::vt::sixel;

mod graphics;
mod images;
mod mode_switch;
mod perform;
mod reply;
#[cfg(test)]
mod tests;

/// Something the host should react to, reported through [`Term::drain_events`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// BEL was received.
    Bell,
    /// The window title was set (OSC 0 or OSC 2).
    Title(String),
    /// OSC 52: the application wants `text` put on the clipboard.
    /// `selection` is the OSC 52 target as sent, e.g. `"c"` (clipboard) or
    /// `"p"` (primary); an empty target means `"s0"` in xterm.
    ClipboardStore { selection: String, text: String },
    /// OSC 52 with `?`: the application asks for the clipboard contents.
    /// The terminal never answers it: a reply would let any program, or a
    /// remote host, read the clipboard.
    ClipboardLoad { selection: String },
}

/// Colors reported to OSC 10/11/12 queries. Applications such as vim and
/// fzf use them to tell a light theme from a dark one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReportedColors {
    pub foreground: (u8, u8, u8),
    pub background: (u8, u8, u8),
    pub cursor: (u8, u8, u8),
}

impl Default for ReportedColors {
    fn default() -> Self {
        Self {
            foreground: (0xff, 0xff, 0xff),
            background: (0, 0, 0),
            cursor: (0xff, 0xff, 0xff),
        }
    }
}

/// Cursor position, zero-based, always inside the screen.
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    /// Attributes given to newly printed characters.
    pub(crate) pen: Cell,
    /// Set after printing into the last column: the cursor stays there and
    /// the wrap happens only when the next character arrives (DECAWM).
    pub(crate) pending_wrap: bool,
    /// G0 and G1, designated by `ESC ( F` and `ESC ) F`.
    charsets: [Charset; 2],
    /// Which of `charsets` is invoked into GL: 0 after SI, 1 after SO.
    gl: usize,
}

/// Character sets that can be designated to G0/G1. Only the DEC line
/// drawing set is in practical use (ncurses, tmux, dialog borders); the
/// national replacement sets are not worth supporting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Charset {
    #[default]
    Ascii,
    DecSpecialGraphics,
}

impl Charset {
    fn map(self, c: char) -> char {
        match (self, c) {
            (Self::DecSpecialGraphics, '_'..='~') => {
                const TABLE: [char; 32] = [
                    ' ', '◆', '▒', '␉', '␌', '␍', '␊', '°', '±', '␤', '␋', '┘', '┐', '┌', '└', '┼',
                    '⎺', '⎻', '─', '⎼', '⎽', '├', '┤', '┴', '┬', '│', '≤', '≥', 'π', '≠', '£', '·',
                ];
                TABLE[c as usize - '_' as usize]
            }
            _ => c,
        }
    }
}

/// What DECSC saves besides the cursor itself.
#[derive(Clone, Debug)]
struct SavedCursor {
    cursor: Cursor,
    origin: bool,
}

/// The terminal state machine. Implements [`vte::Perform`]; the parser is
/// owned by the terminal and driven by [`Term::advance`].
pub struct Term {
    // vte buffers a whole OSC before dispatching it, without a bound: its
    // `Parser<N>` limit only exists without the `std` feature, as an inline
    // array of N bytes, and OSC 1337 inline images need several MiB. So
    // `feed` cuts OSCs off at MAX_OSC_BYTES instead.
    parser: Parser,
    osc: OscScan,
    /// Set while the OSC being parsed must not be dispatched.
    osc_rejected: bool,
    grid: Grid,
    cursor: Cursor,
    modes: Modes,
    cursor_style: CursorStyle,
    /// Rows affected by line feeds and scrolling, set by DECSTBM.
    scroll_region: Range<usize>,
    /// Bytes the terminal sends back to the application (e.g. replies to
    /// status queries).
    responses: Vec<u8>,
    events: VecDeque<Event>,
    tabstops: Vec<bool>,
    /// Saved by DECSC, restored by DECRC. Each screen has its own slot, as
    /// in xterm.
    saved_cursor: Option<SavedCursor>,
    /// The screen not shown: the primary screen while the alternate screen
    /// is active, and vice versa. Swapped with `grid` (and the saved cursor
    /// with `inactive_saved_cursor`) on every switch.
    inactive: Grid,
    inactive_saved_cursor: Option<SavedCursor>,
    /// Kitty keyboard flags, swapped with `inactive_keyboard` like the saved
    /// cursor.
    keyboard: KeyboardStack,
    inactive_keyboard: KeyboardStack,
    alt_screen: bool,
    title: String,
    reported_colors: ReportedColors,
    images: ImageStore,
    /// Physical pixels per cell, to size images given in pixels.
    cell_pixels: (u32, u32),
    /// The Sixel image being received, between DCS `q` and ST.
    sixel: Option<Box<sixel::Decoder>>,
    apc: ApcScanner,
    kitty: graphics::Kitty,
}

const TAB_WIDTH: usize = 8;

/// Longest OSC passed to vte, in bytes: room for an OSC 1337 image of
/// 24 MiB. Longer ones are dropped whole.
const MAX_OSC_BYTES: usize = 32 << 20;
/// Parameters vte keeps per OSC; it silently drops the bytes after the
/// 16th `;`, so such OSCs are dropped whole rather than acted on cut.
const MAX_OSC_PARAMS: usize = 16;

/// Where the fed bytes stand with respect to OSC strings. Tracked beside
/// vte because vte has no hook to bound or cancel an OSC.
#[derive(Clone, Copy, Debug)]
enum OscScan {
    Other,
    /// Right after ESC.
    Escape,
    Osc {
        len: usize,
        params: usize,
    },
    /// The rest of an OSC cut off at [`MAX_OSC_BYTES`], not given to vte.
    Discard,
}

/// Longest window title (OSC 0/2) kept, in bytes.
const MAX_TITLE_BYTES: usize = 4096;
/// Longest text an OSC 52 store may put on the clipboard, in bytes. Longer
/// stores are dropped whole; hosts can lower the limit but not raise it.
pub const MAX_CLIPBOARD_BYTES: usize = 1 << 20;
/// Checked before decoding so that an over-long payload is not decoded.
const MAX_CLIPBOARD_BASE64: usize = MAX_CLIPBOARD_BYTES.div_ceil(3) * 4;

fn default_tabstop(col: usize) -> bool {
    col.is_multiple_of(TAB_WIDTH)
}

/// Keeps a saved cursor on the same text after a resize dropped `dropped`
/// rows from the top, and inside the new size. Clamping only in DECRC is not
/// enough: the next resize of the inactive screen keeps the saved row
/// visible, so that row must exist.
fn clamp_saved_cursor(saved: &mut Option<SavedCursor>, dropped: usize, cols: usize, rows: usize) {
    if let Some(saved) = saved {
        let cursor = &mut saved.cursor;
        cursor.row = cursor.row.saturating_sub(dropped).min(rows - 1);
        cursor.col = cursor.col.min(cols - 1);
    }
}

/// Parameter `i` of a CSI sequence, or `default` when it is absent or zero:
/// for almost every sequence 0 means "use the default".
fn arg(params: &Params, i: usize, default: usize) -> usize {
    match params.iter().nth(i).map(|p| p[0]) {
        None | Some(0) => default,
        Some(v) => usize::from(v),
    }
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            parser: Parser::new(),
            osc: OscScan::Other,
            osc_rejected: false,
            grid: Grid::new(cols, rows),
            cursor: Cursor::default(),
            modes: Modes::default(),
            cursor_style: CursorStyle::default(),
            scroll_region: 0..rows,
            responses: Vec::new(),
            events: VecDeque::new(),
            tabstops: (0..cols).map(default_tabstop).collect(),
            saved_cursor: None,
            // The alternate screen has no scrollback: full-screen
            // applications redraw it themselves.
            inactive: Grid::with_scrollback(cols, rows, 0),
            inactive_saved_cursor: None,
            keyboard: KeyboardStack::default(),
            inactive_keyboard: KeyboardStack::default(),
            alt_screen: false,
            title: String::new(),
            reported_colors: ReportedColors::default(),
            images: ImageStore::default(),
            cell_pixels: (8, 16),
            sixel: None,
            apc: ApcScanner::default(),
            kitty: graphics::Kitty::default(),
        }
    }

    /// Feeds output from the application. Sequences split across calls are
    /// handled.
    pub fn advance(&mut self, bytes: &[u8]) {
        // The parser needs `&mut self` as the performer while it is itself a
        // field of `self`, so move it out for the duration of the call.
        // Keeping the parser outside `Term` would force every caller to
        // carry two objects around.
        let mut parser = std::mem::replace(&mut self.parser, Parser::new());
        // Moved out like the parser: RIS in the middle of `bytes` replaces
        // `self`, but the scanner has already seen the bytes after it.
        let mut apc = std::mem::take(&mut self.apc);
        let mut rest = bytes;
        while !rest.is_empty() {
            // vte ignores APCs, so it gets the bytes up to the end of each
            // kitty APC before the command runs at the cursor it leaves.
            let (n, body) = apc.scan(rest);
            self.feed(&mut parser, &rest[..n]);
            rest = &rest[n..];
            if let Some(body) = body {
                self.kitty_command(&body);
            }
        }
        self.apc = apc;
        self.parser = parser;
    }

    /// Gives `bytes` to vte, rejecting OSCs over [`MAX_OSC_BYTES`] or with
    /// more than [`MAX_OSC_PARAMS`] parameters.
    fn feed(&mut self, parser: &mut Parser, bytes: &[u8]) {
        let mut start = 0;
        for (i, &b) in bytes.iter().enumerate() {
            self.osc = match (self.osc, b) {
                (OscScan::Discard, 0x07 | 0x18 | 0x1a) => {
                    start = i + 1;
                    OscScan::Other
                }
                (OscScan::Discard, 0x1b) => {
                    start = i;
                    OscScan::Escape
                }
                (OscScan::Discard, _) => {
                    start = i + 1;
                    OscScan::Discard
                }
                (OscScan::Osc { .. }, 0x07 | 0x18 | 0x1a) => OscScan::Other,
                (_, 0x1b) => OscScan::Escape,
                (OscScan::Escape, b']') => {
                    // The previous OSC must be dispatched before its flag
                    // is cleared.
                    parser.advance(self, &bytes[start..=i]);
                    start = i + 1;
                    self.osc_rejected = false;
                    OscScan::Osc { len: 0, params: 1 }
                }
                // vte runs C0 controls inside an escape without leaving it.
                (OscScan::Escape, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f) => OscScan::Escape,
                (OscScan::Osc { len, params }, _) if len < MAX_OSC_BYTES => {
                    let params = params + usize::from(b == b';');
                    self.osc_rejected |= params > MAX_OSC_PARAMS;
                    OscScan::Osc {
                        len: len + 1,
                        params,
                    }
                }
                (OscScan::Osc { .. }, _) => {
                    parser.advance(self, &bytes[start..i]);
                    // vte has no abort: CAN ends the OSC by dispatching what
                    // it has, which `osc_rejected` makes a no-op.
                    self.osc_rejected = true;
                    parser.advance(self, &[0x18]);
                    start = i + 1;
                    OscScan::Discard
                }
                _ => OscScan::Other,
            };
        }
        parser.advance(self, &bytes[start..]);
    }

    /// Resizes the screen without reflowing text.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let dropped = self.grid.resize(cols, rows, self.cursor.row);
        self.cursor.row -= dropped;
        clamp_saved_cursor(&mut self.saved_cursor, dropped, cols, rows);
        // The inactive screen's cursor is only known through its saved
        // cursor (1049); keep that row visible.
        let keep = self
            .inactive_saved_cursor
            .as_ref()
            .map_or(0, |s| s.cursor.row);
        let dropped = self.inactive.resize(cols, rows, keep);
        clamp_saved_cursor(&mut self.inactive_saved_cursor, dropped, cols, rows);
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.cursor.pending_wrap = false;
        self.scroll_region = 0..rows;
        let old_cols = self.tabstops.len();
        self.tabstops.truncate(cols);
        self.tabstops.extend((old_cols..cols).map(default_tabstop));
    }

    pub fn cols(&self) -> usize {
        self.grid.cols()
    }

    pub fn rows(&self) -> usize {
        self.grid.rows()
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Panics if `row` or `col` is outside the screen.
    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        self.grid.cell(row, col)
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    pub fn modes(&self) -> &Modes {
        &self.modes
    }

    /// The kitty keyboard protocol flags of the screen shown.
    pub fn keyboard_flags(&self) -> KeyboardFlags {
        self.keyboard.current()
    }

    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    /// Whether the alternate screen (used by full-screen applications such
    /// as vim and less) is shown. It has no scrollback, so hosts usually turn
    /// the mouse wheel into arrow keys while it is active.
    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }

    /// Returns the display lines (see [`Grid`]) changed since the previous
    /// call. Cursor movement alone does not make a line dirty.
    pub fn take_dirty(&mut self) -> Vec<usize> {
        self.grid.take_dirty()
    }

    /// Display line `line`: scrollback while the view is scrolled back,
    /// otherwise the screen.
    pub fn display_row(&self, line: usize) -> &Row {
        self.grid.display_row(line)
    }

    /// How many rows the view is scrolled back; 0 shows the live screen.
    /// The cursor is on display line `cursor().row + display_offset()`,
    /// which is off the view when that is `rows()` or more.
    pub fn display_offset(&self) -> usize {
        self.grid.display_offset()
    }

    /// Scrolls the view back into the scrollback (`lines > 0`) or towards
    /// the live screen (`lines < 0`), clamped at both ends.
    pub fn scroll_display(&mut self, lines: isize) {
        let offset = self.grid.display_offset().saturating_add_signed(lines);
        self.grid.set_display_offset(offset);
    }

    /// Shows scrollback starting `offset` rows above the live screen.
    pub fn set_display_offset(&mut self, offset: usize) {
        self.grid.set_display_offset(offset);
    }

    pub fn scrollback_len(&self) -> usize {
        self.grid.scrollback_len()
    }

    /// See [`Grid::scrollback_dropped`]; counts the primary screen's
    /// scrollback only while it is shown.
    pub fn scrollback_dropped(&self) -> u64 {
        self.grid.scrollback_dropped()
    }

    /// Changes how many rows of scrollback are kept (default
    /// [`DEFAULT_SCROLLBACK`](crate::vt::DEFAULT_SCROLLBACK)); excess rows are
    /// dropped, oldest first. 0 disables the scrollback. Applies to the
    /// primary screen even while the alternate screen is shown.
    pub fn set_scrollback_limit(&mut self, limit: usize) {
        let primary = if self.alt_screen {
            &mut self.inactive
        } else {
            &mut self.grid
        };
        primary.set_scrollback_limit(limit);
    }

    /// Takes the bytes that must be written back to the application.
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    /// The last title set by the application.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Sets the colors answered to OSC 10/11/12 queries; the terminal does
    /// not otherwise know the theme the renderer uses.
    pub fn set_reported_colors(&mut self, colors: ReportedColors) {
        self.reported_colors = colors;
    }

    /// Tells the terminal the size of a cell in pixels, which it cannot know
    /// itself; images sized in pixels are converted to cells with it.
    pub fn set_cell_pixel_size(&mut self, width: u32, height: u32) {
        self.cell_pixels = (width.max(1), height.max(1));
    }

    pub fn cell_pixel_size(&self) -> (u32, u32) {
        self.cell_pixels
    }

    /// Caps the decoded image data kept for display (default
    /// [`DEFAULT_IMAGE_MEMORY_LIMIT`](crate::vt::DEFAULT_IMAGE_MEMORY_LIMIT)).
    /// Beyond it the oldest images are removed from the grid.
    pub fn set_image_memory_limit(&mut self, bytes: usize) {
        self.images.set_limit(bytes);
        let evict = self.images.over_limit();
        self.remove_images(&evict);
    }

    /// Shows `image` with its top-left corner at the cursor. The covered
    /// cells are erased and attached to the image, so it scrolls with them
    /// and is removed piece by piece when they are overwritten or erased.
    /// Columns past the right edge are cut; rows past the bottom scroll the
    /// screen (or are cut with [`ImageCursor::Stay`]).
    pub fn place_image(&mut self, image: Arc<Image>, placement: Placement) {
        self.put_image(image, placement);
    }

    /// Events since the previous call, oldest first.
    pub fn drain_events(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.events.drain(..)
    }

    fn linefeed(&mut self) {
        self.cursor.pending_wrap = false;
        let bottom = self.scroll_region.end - 1;
        if self.cursor.row == bottom {
            self.grid
                .scroll_up(self.scroll_region.clone(), 1, self.cursor.pen.bg);
        } else if self.cursor.row + 1 < self.rows() {
            self.cursor.row += 1;
        }
    }

    /// Moves the cursor to an absolute position, clamped to the screen.
    fn goto(&mut self, row: usize, col: usize) {
        self.cursor.row = row.min(self.rows() - 1);
        self.cursor.col = col.min(self.cols() - 1);
        self.cursor.pending_wrap = false;
    }

    fn goto_col(&mut self, col: usize) {
        self.goto(self.cursor.row, col);
    }

    /// Moves the cursor as CUP and VPA do: under DECOM the row counts from
    /// the top of the scroll region and stays inside it.
    fn goto_addressed(&mut self, row: usize, col: usize) {
        let row = if self.modes.origin {
            (self.scroll_region.start + row).min(self.scroll_region.end - 1)
        } else {
            row
        };
        self.goto(row, col);
    }

    /// DECSTBM. A region of fewer than two rows is ignored, as in xterm.
    fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.rows());
        if top + 1 >= bottom {
            return;
        }
        self.scroll_region = top..bottom;
        self.goto_addressed(0, 0);
    }

    /// SU (`up`) and SD: scroll the region without moving the cursor.
    fn scroll_region_by(&mut self, n: usize, up: bool) {
        let region = self.scroll_region.clone();
        let bg = self.cursor.pen.bg;
        if up {
            self.grid.scroll_up(region, n, bg);
        } else {
            self.grid.scroll_down(region, n, bg);
        }
    }

    /// Limits for relative vertical movement: the scroll region while the
    /// cursor is inside it, otherwise the whole screen.
    fn vertical_bounds(&self) -> (usize, usize) {
        if self.scroll_region.contains(&self.cursor.row) {
            (self.scroll_region.start, self.scroll_region.end - 1)
        } else {
            (0, self.rows() - 1)
        }
    }

    fn move_up(&mut self, n: usize) {
        let (top, _) = self.vertical_bounds();
        let row = self.cursor.row.saturating_sub(n).max(top);
        self.goto(row, self.cursor.col);
    }

    fn move_down(&mut self, n: usize) {
        let (_, bottom) = self.vertical_bounds();
        let row = (self.cursor.row + n).min(bottom);
        self.goto(row, self.cursor.col);
    }

    fn reverse_index(&mut self) {
        self.cursor.pending_wrap = false;
        if self.cursor.row == self.scroll_region.start {
            self.grid
                .scroll_down(self.scroll_region.clone(), 1, self.cursor.pen.bg);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    fn tab_forward(&mut self, n: usize) {
        let mut col = self.cursor.col;
        for _ in 0..n {
            col = (col + 1..self.cols())
                .find(|&c| self.tabstops[c])
                .unwrap_or(self.cols() - 1);
        }
        self.goto_col(col);
    }

    fn tab_backward(&mut self, n: usize) {
        let mut col = self.cursor.col;
        for _ in 0..n {
            col = (0..col).rev().find(|&c| self.tabstops[c]).unwrap_or(0);
        }
        self.goto_col(col);
    }

    /// ED.
    fn erase_display(&mut self, mode: u16) {
        let row = self.cursor.row;
        let bg = self.cursor.pen.bg;
        self.cursor.pending_wrap = false;
        let rows = match mode {
            0 => {
                self.erase_line(0);
                row + 1..self.rows()
            }
            1 => {
                self.erase_line(1);
                0..row
            }
            2 => 0..self.rows(),
            3 => {
                self.grid.clear_scrollback();
                return;
            }
            _ => return,
        };
        for r in rows {
            self.grid.row_mut(r).reset(bg);
        }
    }

    /// EL.
    fn erase_line(&mut self, mode: u16) {
        let (row, col) = (self.cursor.row, self.cursor.col);
        let bg = self.cursor.pen.bg;
        let cols = self.cols();
        self.cursor.pending_wrap = false;
        let line = self.grid.row_mut(row);
        match mode {
            0 => {
                line.erase(col..cols, bg);
                line.set_wrapped(false);
            }
            1 => line.erase(0..col + 1, bg),
            2 => line.reset(bg),
            _ => {}
        }
    }

    /// IL (`insert`) and DL: only inside the scroll region, and the cursor
    /// returns to the first column as in xterm.
    fn insert_or_delete_lines(&mut self, n: usize, insert: bool) {
        let row = self.cursor.row;
        if !self.scroll_region.contains(&row) {
            return;
        }
        let region = row..self.scroll_region.end;
        let bg = self.cursor.pen.bg;
        if insert {
            self.grid.scroll_down(region, n, bg);
        } else {
            self.grid.scroll_up(region, n, bg);
        }
        self.goto_col(0);
    }

    /// Before overwriting `width` cells at the cursor, blanks the other half
    /// of any wide character that would be cut in two.
    fn clear_wide_halves(&mut self, row: usize, col: usize, width: usize) {
        let cols = self.cols();
        let bg = self.cursor.pen.bg;
        if col > 0 && self.grid.cell(row, col).flags.contains(Flags::WIDE_SPACER) {
            *self.grid.cell_mut(row, col - 1) = Cell::blank(bg);
        }
        let last = col + width - 1;
        if last + 1 < cols && self.grid.cell(row, last).flags.contains(Flags::WIDE) {
            *self.grid.cell_mut(row, last + 1) = Cell::blank(bg);
        }
    }

    /// The cell the last printed character went to: the one before the
    /// cursor, or under it while a wrap is pending, skipping a wide spacer.
    fn previous_cell(&self) -> Option<(usize, usize)> {
        let row = self.cursor.row;
        let mut col = if self.cursor.pending_wrap {
            self.cursor.col
        } else if self.cursor.col > 0 {
            self.cursor.col - 1
        } else {
            return None;
        };
        if self.grid.cell(row, col).flags.contains(Flags::WIDE_SPACER) && col > 0 {
            col -= 1;
        }
        Some((row, col))
    }

    fn print_zerowidth(&mut self, c: char) {
        if let Some((row, col)) = self.previous_cell() {
            self.grid.cell_mut(row, col).push_zerowidth(c);
        }
    }

    /// Mode 2027: adds `c` to the cell before the cursor when UAX #29 puts
    /// no grapheme cluster boundary between that cell's text and `c`.
    /// Returns whether it did.
    ///
    /// The cluster is read back from the cell rather than kept as parser
    /// state, so cursor movement and overwrites need no bookkeeping.
    fn extend_cluster(&mut self, c: char) -> bool {
        let Some((row, col)) = self.previous_cell() else {
            return false;
        };
        let cell = self.grid.cell(row, col);
        // An erased cell is also a space, so a lone skin tone modifier after
        // a blank would vanish into it. Why not join there anyway, as UAX #29
        // allows: only zero-width characters are worth keeping on a blank.
        if cell.c == ' ' && c.width() != Some(0) {
            return false;
        }
        let mut buf = [0u8; 4 * (MAX_ZEROWIDTH + 2)];
        let mut len = 0;
        let chars = std::iter::once(cell.c)
            .chain(cell.zerowidth().iter().copied())
            .chain(std::iter::once(c));
        for ch in chars {
            len += ch.encode_utf8(&mut buf[len..]).len();
        }
        let text = std::str::from_utf8(&buf[..len]).expect("encoded from chars");
        if text.graphemes(true).nth(1).is_some() {
            return false;
        }
        let widen = text.width() >= 2 && !cell.flags.contains(Flags::WIDE);
        self.grid.cell_mut(row, col).push_zerowidth(c);
        if widen {
            self.widen(row, col);
        }
        true
    }

    /// Mode 2027: a narrow cluster that became two cells wide (VS16, a flag,
    /// a keycap) takes the next cell as its spacer, moving to the next row
    /// first if it sits in the last column. Why not the other way round
    /// (VS15 narrowing a wide one): the Contour proposal keeps the width for
    /// VS15, and shrinking would leave a hole after the cluster.
    fn widen(&mut self, row: usize, col: usize) {
        let cols = self.cols();
        if cols < 2 {
            return;
        }
        let (row, col) = if col + 1 < cols {
            (row, col)
        } else if self.modes.autowrap {
            let bg = self.cursor.pen.bg;
            let cell = std::mem::replace(self.grid.cell_mut(row, col), Cell::blank(bg));
            self.cursor.pending_wrap = false;
            self.wrap();
            let row = self.cursor.row;
            self.clear_wide_halves(row, 0, 1);
            *self.grid.cell_mut(row, 0) = cell;
            (row, 0)
        } else {
            // Why not step back a column, as a wide character does with
            // DECAWM reset: that would overwrite the character before it.
            return;
        };
        if self.modes.insert {
            let bg = self.cursor.pen.bg;
            self.grid.row_mut(row).insert_blank(col + 1, 1, bg);
        }
        self.clear_wide_halves(row, col + 1, 1);
        let pen = self.cursor.pen.clone();
        self.grid.cell_mut(row, col).flags.insert(Flags::WIDE);
        let spacer = self.grid.cell_mut(row, col + 1);
        spacer.write(' ', &pen);
        spacer.flags.insert(Flags::WIDE_SPACER);
        if col + 2 < cols {
            self.cursor.col = col + 2;
            self.cursor.pending_wrap = false;
        } else {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = true;
        }
    }

    /// DECSC.
    fn save_cursor(&mut self) {
        self.saved_cursor = Some(SavedCursor {
            cursor: self.cursor.clone(),
            origin: self.modes.origin,
        });
    }

    /// DECRC. Without a prior DECSC the cursor goes home with default
    /// attributes, as in xterm.
    fn restore_cursor(&mut self) {
        let saved = self.saved_cursor.clone().unwrap_or(SavedCursor {
            cursor: Cursor::default(),
            origin: false,
        });
        self.cursor = saved.cursor;
        self.modes.origin = saved.origin;
        // Not `goto`, which would drop the saved pending wrap; the screen
        // may have shrunk and the region may have moved since the save,
        // though. Under DECOM xterm puts the cursor back inside the region.
        self.cursor.row = if self.modes.origin {
            let region = &self.scroll_region;
            self.cursor.row.clamp(region.start, region.end - 1)
        } else {
            self.cursor.row.min(self.rows() - 1)
        };
        self.cursor.col = self.cursor.col.min(self.cols() - 1);
    }

    fn clear_screen(&mut self, bg: Color) {
        for r in 0..self.rows() {
            self.grid.row_mut(r).reset(bg);
        }
    }

    /// RIS: back to the power-on state, keeping only the size, the
    /// scrollback limit and anything not yet taken by the host.
    fn full_reset(&mut self) {
        let mut fresh = Term::new(self.cols(), self.rows());
        let primary = if self.alt_screen {
            &self.inactive
        } else {
            &self.grid
        };
        fresh.grid.set_scrollback_limit(primary.scrollback_limit());
        fresh.grid.continue_dropped_count(primary);
        fresh.responses = std::mem::take(&mut self.responses);
        fresh.events = std::mem::take(&mut self.events);
        fresh.title = std::mem::take(&mut self.title);
        fresh.reported_colors = self.reported_colors;
        // Set by the host, which does not repeat them after a reset.
        fresh.cell_pixels = self.cell_pixels;
        fresh.images.set_limit(self.images.limit());
        *self = fresh;
    }

    /// DECSTR: resets modes and attributes but leaves the screen contents.
    fn soft_reset(&mut self) {
        // Mouse, paste, focus and screen modes survive DECSTR, as in xterm.
        self.modes = Modes {
            mouse_tracking: self.modes.mouse_tracking,
            mouse_encoding: self.modes.mouse_encoding,
            bracketed_paste: self.modes.bracketed_paste,
            focus_reporting: self.modes.focus_reporting,
            reverse_video: self.modes.reverse_video,
            synchronized_output: self.modes.synchronized_output,
            ..Modes::default()
        };
        self.cursor_style = CursorStyle::default();
        self.scroll_region = 0..self.rows();
        self.cursor.pen = Cell::default();
        self.cursor.pending_wrap = false;
        self.cursor.charsets = Default::default();
        self.cursor.gl = 0;
        self.saved_cursor = None;
    }

    /// OSC 8 `params ; uri`. An empty URI ends the link.
    fn set_hyperlink(&mut self, params: &[u8], uri: &[u8]) {
        let link = (!uri.is_empty()).then(|| {
            let id = params
                .split(|&b| b == b':')
                .find_map(|kv| kv.strip_prefix(b"id="))
                .map(|id| String::from_utf8_lossy(id).into_owned());
            Arc::new(Hyperlink {
                id,
                uri: String::from_utf8_lossy(uri).into_owned(),
            })
        });
        self.cursor.pen.set_hyperlink(link);
    }

    fn wrap(&mut self) {
        self.grid.row_mut(self.cursor.row).set_wrapped(true);
        self.linefeed();
        self.cursor.col = 0;
    }
}
