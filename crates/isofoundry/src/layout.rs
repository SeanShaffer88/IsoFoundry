//! Main-window geometry in physical pixels, computed from the client size
//! and the DPI scale. Pure math so it can be tested on any platform.
//!
//! Base units are 96-DPI pixels taken from the approved mockup (D-045).

use if_core::tasks::GRID;

pub const MARGIN_X: i32 = 24;
pub const MARGIN_TOP: i32 = 18;
pub const GROUP_PAD_X: i32 = 14;
pub const LABEL_TOP: i32 = 12;
pub const LABEL_H: i32 = 18;
pub const LABEL_GAP: i32 = 10;
pub const ROW_H: i32 = 32;
pub const ROW_GAP: i32 = 9;
pub const COL_GAP: i32 = 14;
pub const ARROW_W: i32 = 30;
pub const SPLIT_GAP: i32 = 2;
pub const GROUP_PAD_BOTTOM: i32 = 16;
pub const FOOTER_MIN_H: i32 = 84;
pub const MIN_CELL_W: i32 = 300;
pub const DEFAULT_CLIENT_W: i32 = 816;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self { left, top, right: left + width, bottom: top + height }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    #[cfg(test)]
    pub fn intersects(&self, o: &Rect) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
}

/// One task button; `arrow` is the split-button menu part, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
    pub main: Rect,
    pub arrow: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub group: Rect,
    pub label: Rect,
    pub cells: Vec<Cell>,
    pub footer: Rect,
}

/// Scales a 96-DPI length, rounding to the nearest pixel.
pub fn px(v: i32, dpi: u32) -> i32 {
    ((v as i64 * dpi as i64 + 48) / 96) as i32
}

fn rows() -> i32 {
    GRID.len() as i32
}

fn group_height(dpi: u32) -> i32 {
    px(LABEL_TOP + LABEL_H + LABEL_GAP, dpi)
        + rows() * px(ROW_H, dpi)
        + (rows() - 1) * px(ROW_GAP, dpi)
        + px(GROUP_PAD_BOTTOM, dpi)
}

/// Smallest client area that shows everything without clipping or scrollbars.
pub fn min_client(dpi: u32) -> (i32, i32) {
    let w = 2 * px(MARGIN_X, dpi) + 2 * px(GROUP_PAD_X, dpi) + 2 * px(MIN_CELL_W, dpi) + px(COL_GAP, dpi);
    let h = px(MARGIN_TOP, dpi) + group_height(dpi) + px(FOOTER_MIN_H, dpi);
    (w, h)
}

/// Default client size: the mockup width, minimum height plus a little room for the logo.
pub fn default_client(dpi: u32) -> (i32, i32) {
    let (mw, mh) = min_client(dpi);
    (mw.max(px(DEFAULT_CLIENT_W, dpi)), mh + px(16, dpi))
}

/// Lays out the grid. Columns stretch with the window; rows keep their
/// height and extra height goes to the footer.
pub fn compute(client_w: i32, client_h: i32, dpi: u32) -> Layout {
    let (min_w, min_h) = min_client(dpi);
    let cw = client_w.max(min_w);
    let ch = client_h.max(min_h);

    let gx = px(MARGIN_X, dpi);
    let gy = px(MARGIN_TOP, dpi);
    let group = Rect::new(gx, gy, cw - 2 * gx, group_height(dpi));
    let inner_l = group.left + px(GROUP_PAD_X, dpi);
    let inner_r = group.right - px(GROUP_PAD_X, dpi);
    let label = Rect::new(inner_l, group.top + px(LABEL_TOP, dpi), inner_r - inner_l, px(LABEL_H, dpi));

    let col_gap = px(COL_GAP, dpi);
    let col_w = (inner_r - inner_l - col_gap) / 2;
    let row_h = px(ROW_H, dpi);
    let row_step = row_h + px(ROW_GAP, dpi);
    let first = label.bottom + px(LABEL_GAP, dpi);
    let arrow_w = px(ARROW_W, dpi);
    let split_gap = px(SPLIT_GAP, dpi);

    let mut cells = Vec::new();
    for (r, row) in GRID.iter().enumerate() {
        for (c, slot) in row.iter().enumerate() {
            let Some(task) = slot else { continue };
            let left = inner_l + c as i32 * (col_w + col_gap);
            // The right column absorbs the odd pixel so both edges line up.
            let width = if c == 1 { inner_r - left } else { col_w };
            let top = first + r as i32 * row_step;
            let cell = if task.variants.is_empty() {
                Cell { row: r, col: c, main: Rect::new(left, top, width, row_h), arrow: None }
            } else {
                let main_w = width - arrow_w - split_gap;
                Cell {
                    row: r,
                    col: c,
                    main: Rect::new(left, top, main_w, row_h),
                    arrow: Some(Rect::new(left + main_w + split_gap, top, arrow_w, row_h)),
                }
            };
            cells.push(cell);
        }
    }
    let footer = Rect { left: 0, top: group.bottom, right: cw, bottom: ch };
    Layout { group, label, cells, footer }
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_core::tasks;

    #[test]
    fn scaling_rounds() {
        assert_eq!(px(32, 96), 32);
        assert_eq!(px(32, 144), 48);
        assert_eq!(px(30, 120), 38); // 37.5 rounds up
    }

    #[test]
    fn one_cell_per_task() {
        let l = compute(816, 640, 96);
        assert_eq!(l.cells.len(), tasks::all().count());
        let splits = tasks::all().filter(|t| !t.variants.is_empty()).count();
        assert_eq!(l.cells.iter().filter(|c| c.arrow.is_some()).count(), splits);
    }

    #[test]
    fn nothing_overlaps_and_everything_fits() {
        for dpi in [96, 120, 144, 192] {
            let (w, h) = min_client(dpi);
            let l = compute(w, h, dpi);
            let mut rects: Vec<Rect> = Vec::new();
            for c in &l.cells {
                rects.push(c.main);
                rects.extend(c.arrow);
            }
            for (i, a) in rects.iter().enumerate() {
                assert!(a.width() > 0 && a.height() > 0);
                assert!(a.left >= l.group.left && a.right <= l.group.right, "dpi {dpi}");
                assert!(a.top >= l.label.bottom && a.bottom <= l.group.bottom, "dpi {dpi}");
                for b in &rects[i + 1..] {
                    assert!(!a.intersects(b), "dpi {dpi}: {a:?} overlaps {b:?}");
                }
            }
            assert!(l.group.right <= w && l.footer.bottom <= h.max(l.footer.bottom));
            assert!(l.footer.height() >= px(FOOTER_MIN_H, dpi));
        }
    }

    #[test]
    fn columns_line_up_and_stretch() {
        let small = compute(816, 640, 96);
        let big = compute(1400, 900, 96);
        let right_edge =
            |l: &Layout| l.cells.iter().filter(|c| c.col == 1).map(|c| c.arrow.unwrap_or(c.main).right).max();
        let r = right_edge(&small).unwrap();
        assert!(small.cells.iter().filter(|c| c.col == 1).all(|c| c.arrow.unwrap_or(c.main).right == r));
        assert!(right_edge(&big).unwrap() > r);
        // Row height never stretches; the footer takes the extra height.
        assert_eq!(big.cells[0].main.height(), small.cells[0].main.height());
        assert!(big.footer.height() > small.footer.height());
    }

    #[test]
    fn below_minimum_is_clamped() {
        let l = compute(10, 10, 96);
        let (w, _) = min_client(96);
        assert_eq!(l.group.right, w - px(MARGIN_X, 96));
    }

    #[test]
    fn default_is_mockup_width() {
        assert_eq!(default_client(96).0, 816);
        assert!(default_client(192).0 >= 1632);
    }
}
