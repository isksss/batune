use std::collections::{HashSet, VecDeque};
use std::ops::{Index, Range};

use crate::vt::cell::{Cell, Color, Flags};
use crate::vt::image::ImageCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    cells: Vec<Cell>,
    wrapped: bool,
}

impl Row {
    fn new(cols: usize, bg: Color) -> Self {
        Self {
            cells: vec![Cell::blank(bg); cols],
            wrapped: false,
        }
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Whether the text continues on the next row because of auto-wrap
    /// (as opposed to an explicit line break). Used for copy and reflow.
    pub fn wrapped(&self) -> bool {
        self.wrapped
    }

    pub(crate) fn set_wrapped(&mut self, wrapped: bool) {
        self.wrapped = wrapped;
    }

    /// The row's text with wide-character spacers skipped and trailing
    /// blanks trimmed.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for cell in &self.cells {
            if cell.flags.contains(Flags::WIDE_SPACER) {
                continue;
            }
            s.push(cell.c);
            s.extend(cell.zerowidth());
        }
        s.truncate(s.trim_end_matches(' ').len());
        s
    }

    pub(crate) fn reset(&mut self, bg: Color) {
        self.cells.fill(Cell::blank(bg));
        self.wrapped = false;
    }

    /// Blanks `cols`, keeping kitty placements; [`Row::reset`] removes them.
    pub(crate) fn erase(&mut self, cols: Range<usize>, bg: Color) {
        for cell in &mut self.cells[cols] {
            cell.erase(bg);
        }
        self.repair_wide(bg);
    }

    /// Shifts cells from `col` right by `n`; cells pushed past the edge are lost.
    pub(crate) fn insert_blank(&mut self, col: usize, n: usize, bg: Color) {
        self.keep_layer(|cells| {
            let n = n.min(cells.len() - col);
            cells[col..].rotate_right(n);
            cells[col..col + n].fill(Cell::blank(bg));
        });
        self.repair_wide(bg);
    }

    /// Removes `n` cells at `col`, pulling the rest left and filling the end
    /// with blanks.
    pub(crate) fn delete(&mut self, col: usize, n: usize, bg: Color) {
        self.keep_layer(|cells| {
            let len = cells.len();
            let n = n.min(len - col);
            cells[col..].rotate_left(n);
            cells[len - n..].fill(Cell::blank(bg));
        });
        self.repair_wide(bg);
    }

    /// Runs `shift` and puts kitty placements back in the columns they had:
    /// in kitty the text moves under them.
    fn keep_layer(&mut self, shift: impl FnOnce(&mut [Cell])) {
        if !self
            .cells
            .iter()
            .any(|c| c.image().is_some_and(ImageCell::layered))
        {
            return shift(&mut self.cells);
        }
        let layer: Vec<_> = self.cells.iter().map(Cell::layered_image).collect();
        shift(&mut self.cells);
        for (cell, image) in self.cells.iter_mut().zip(layer) {
            if image.is_some() || cell.image().is_some_and(ImageCell::layered) {
                cell.set_image(image);
            }
        }
    }

    /// Blanks halves of wide characters whose other half was erased or
    /// shifted away. A full scan is simpler than special-casing each edge of
    /// each edit, and rows are short.
    fn repair_wide(&mut self, bg: Color) {
        let len = self.cells.len();
        for i in 0..len {
            let flags = self.cells[i].flags;
            let orphan_head = flags.contains(Flags::WIDE)
                && !(i + 1 < len && self.cells[i + 1].flags.contains(Flags::WIDE_SPACER));
            let orphan_spacer = flags.contains(Flags::WIDE_SPACER)
                && !(i > 0 && self.cells[i - 1].flags.contains(Flags::WIDE));
            if orphan_head || orphan_spacer {
                self.cells[i].erase(bg);
            }
        }
    }

    fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::default());
        self.repair_wide(Color::Default);
    }
}

impl Index<usize> for Row {
    type Output = Cell;

    fn index(&self, col: usize) -> &Cell {
        &self.cells[col]
    }
}

/// Scrollback kept by default, in rows.
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// The screen: `rows` rows of `cols` cells, the rows that scrolled off the
/// top (scrollback), and per-row damage.
///
/// Rows are addressed two ways. *Screen* rows (`row`, `cell`) are what the
/// application writes to. *Display* lines (`display_row`, `take_dirty`)
/// are what the user sees: the screen shifted down by
/// [`display_offset`](Self::display_offset) rows of scrollback.
#[derive(Clone, Debug)]
pub struct Grid {
    rows: Vec<Row>,
    cols: usize,
    dirty: Vec<bool>,
    /// Oldest row first. A `VecDeque` is the ring buffer: rows are pushed at
    /// the back and dropped at the front once the limit is reached.
    scrollback: VecDeque<Row>,
    scrollback_limit: usize,
    /// Rows ever dropped from the front of `scrollback`.
    scrollback_dropped: u64,
    display_offset: usize,
    /// Every display line must be redrawn, e.g. because the view moved.
    all_dirty: bool,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self::with_scrollback(cols, rows, DEFAULT_SCROLLBACK)
    }

    pub(crate) fn with_scrollback(cols: usize, rows: usize, limit: usize) -> Self {
        Self {
            rows: (0..rows).map(|_| Row::new(cols, Color::Default)).collect(),
            cols,
            dirty: vec![true; rows],
            scrollback: VecDeque::new(),
            scrollback_limit: limit,
            scrollback_dropped: 0,
            display_offset: 0,
            all_dirty: true,
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn row(&self, row: usize) -> &Row {
        &self.rows[row]
    }

    /// Mutable access marks the row dirty; there is no other way to change
    /// a row, so damage can never be missed.
    pub(crate) fn row_mut(&mut self, row: usize) -> &mut Row {
        self.dirty[row] = true;
        &mut self.rows[row]
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.rows[row].cells[col]
    }

    pub(crate) fn cell_mut(&mut self, row: usize, col: usize) -> &mut Cell {
        &mut self.row_mut(row).cells[col]
    }

    /// Moves rows in `region` up by `n`; the rows that appear at the bottom
    /// are blank. Rows leaving the top of the screen go to the scrollback,
    /// as in xterm; rows leaving a region that starts lower are lost.
    pub(crate) fn scroll_up(&mut self, region: Range<usize>, n: usize, bg: Color) {
        let n = n.min(region.len());
        if region.start == 0 && self.scrollback_limit > 0 {
            for i in 0..n {
                let blank = self.recycle_row(bg);
                let gone = std::mem::replace(&mut self.rows[i], blank);
                self.scrollback.push_back(gone);
            }
            if self.display_offset > 0 {
                // Keep the user's view on the same text while output arrives.
                self.display_offset = (self.display_offset + n).min(self.scrollback.len());
                self.all_dirty = true;
            }
        } else {
            for row in &mut self.rows[region.start..region.start + n] {
                row.reset(bg);
            }
        }
        self.rows[region.clone()].rotate_left(n);
        self.dirty[region].fill(true);
    }

    /// A blank row for the bottom of the screen, reusing the oldest
    /// scrollback row when the scrollback is full so that a steady stream of
    /// output does not allocate.
    fn recycle_row(&mut self, bg: Color) -> Row {
        if self.scrollback.len() >= self.scrollback_limit
            && let Some(mut row) = self.scrollback.pop_front()
        {
            self.scrollback_dropped += 1;
            row.resize(self.cols);
            row.reset(bg);
            row
        } else {
            Row::new(self.cols, bg)
        }
    }

    /// Removes the pieces of the images with these ids from every cell,
    /// scrollback included.
    pub(crate) fn remove_images(&mut self, ids: &[u64]) {
        self.remove_image_cells(|i| ids.contains(&i.image().id()));
    }

    /// Removes the placements with these [`ImageCell::placement_id`]s.
    pub(crate) fn remove_placements(&mut self, ids: &[u64]) {
        self.remove_image_cells(|i| ids.contains(&i.placement_id()));
    }

    /// Adds the [`ImageCell::placement_id`]s shown anywhere in the grid.
    pub(crate) fn placement_ids(&self, ids: &mut HashSet<u64>) {
        let rows = self.rows.iter().chain(self.scrollback.iter());
        ids.extend(rows.flat_map(|r| {
            r.cells
                .iter()
                .filter_map(|c| Some(c.image()?.placement_id()))
        }));
    }

    fn remove_image_cells(&mut self, remove: impl Fn(&ImageCell) -> bool) {
        for row in self.rows.iter_mut().chain(self.scrollback.iter_mut()) {
            for cell in &mut row.cells {
                if cell.image().is_some_and(&remove) {
                    cell.set_image(None);
                    self.all_dirty = true;
                }
            }
        }
    }

    pub(crate) fn mark_all_dirty(&mut self) {
        self.all_dirty = true;
    }

    /// Rows of scrollback currently kept.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Scrollback row `i`, oldest first.
    pub fn scrollback_row(&self, i: usize) -> &Row {
        &self.scrollback[i]
    }

    pub fn scrollback_limit(&self) -> usize {
        self.scrollback_limit
    }

    /// Rows dropped from the front of the scrollback so far, by the limit,
    /// ED 3 or RIS. Scrollback row `i` is the
    /// `scrollback_dropped() + i`-th row to have entered the scrollback, so
    /// a host can keep positions in it while output arrives.
    pub fn scrollback_dropped(&self) -> u64 {
        self.scrollback_dropped
    }

    pub(crate) fn set_scrollback_limit(&mut self, limit: usize) {
        self.scrollback_limit = limit;
        let excess = self.scrollback.len().saturating_sub(limit);
        self.drop_scrollback(excess);
        self.set_display_offset(self.display_offset);
    }

    /// ED 3.
    pub(crate) fn clear_scrollback(&mut self) {
        self.drop_scrollback(self.scrollback.len());
        self.set_display_offset(0);
    }

    fn drop_scrollback(&mut self, n: usize) {
        self.scrollback.drain(..n);
        self.scrollback_dropped += n as u64;
    }

    /// RIS replaces the grid; the new one carries on the count as if the old
    /// scrollback had been cleared.
    pub(crate) fn continue_dropped_count(&mut self, old: &Grid) {
        self.scrollback_dropped = old.scrollback_dropped + old.scrollback.len() as u64;
    }

    /// How many rows of scrollback the view is shifted up by; 0 shows the
    /// live screen.
    pub fn display_offset(&self) -> usize {
        self.display_offset
    }

    /// Clamped to the scrollback length.
    pub(crate) fn set_display_offset(&mut self, offset: usize) {
        let offset = offset.min(self.scrollback.len());
        if offset != self.display_offset {
            self.display_offset = offset;
            self.all_dirty = true;
        }
    }

    /// Display line `line` (0 is the top of the view).
    pub fn display_row(&self, line: usize) -> &Row {
        if line < self.display_offset {
            &self.scrollback[self.scrollback.len() - self.display_offset + line]
        } else {
            &self.rows[line - self.display_offset]
        }
    }

    /// Moves rows in `region` down by `n`; the rows that appear at the top
    /// are blank.
    pub(crate) fn scroll_down(&mut self, region: Range<usize>, n: usize, bg: Color) {
        let n = n.min(region.len());
        self.rows[region.clone()].rotate_right(n);
        for row in &mut self.rows[region.start..region.start + n] {
            row.reset(bg);
        }
        self.dirty[region].fill(true);
    }

    /// Changes the size without reflowing text. Rows are added or removed at
    /// the bottom, except that `keep_row` is kept visible by dropping rows
    /// from the top instead; those go to the scrollback. Returns how many
    /// rows were dropped from the top.
    pub(crate) fn resize(&mut self, cols: usize, rows: usize, keep_row: usize) -> usize {
        let dropped_top = (keep_row + 1).saturating_sub(rows).min(self.rows.len());
        let dropped: Vec<Row> = self.rows.drain(..dropped_top).collect();
        if self.scrollback_limit > 0 {
            self.scrollback.extend(dropped);
            let excess = self.scrollback.len().saturating_sub(self.scrollback_limit);
            self.drop_scrollback(excess);
        }
        self.rows.truncate(rows);
        self.rows
            .resize_with(rows, || Row::new(self.cols, Color::Default));
        // Scrollback rows are resized too, so that every row a renderer sees
        // has exactly `cols` cells.
        for row in self.rows.iter_mut().chain(self.scrollback.iter_mut()) {
            row.resize(cols);
        }
        self.cols = cols;
        self.dirty = vec![true; rows];
        self.display_offset = 0;
        self.all_dirty = true;
        dropped_top
    }

    /// Returns the display lines changed since the previous call, in
    /// ascending order.
    pub fn take_dirty(&mut self) -> Vec<usize> {
        let len = self.dirty.len();
        let lines = if std::mem::take(&mut self.all_dirty) {
            (0..len).collect()
        } else {
            (0..len)
                .filter(|&i| self.dirty[i])
                .map(|i| i + self.display_offset)
                .filter(|&line| line < len)
                .collect()
        };
        self.dirty.fill(false);
        lines
    }
}
