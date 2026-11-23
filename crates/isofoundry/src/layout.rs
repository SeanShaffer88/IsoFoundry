//! Main-window geometry in physical pixels, computed from the client size
//! and the DPI scale. Pure math so it can be tested on any platform.
//!
//! Base units are 96-DPI pixels taken from the approved mockup (D-045).

use if_core::tasks::GRID;

pub const MARGIN_X: i32 = 24;
pub const MARGIN_TOP: i32 = 12;
pub const HEADER_H: i32 = 52;
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
pub const FOOTER_MIN_H: i32 = 34;
pub const MIN_CELL_W: i32 = 300;
pub const DEFAULT_CLIENT_W: i32 = 816;
pub const DETAILS_W: i32 = 280;
pub const DRAWER_H: i32 = 136;
pub const NAV_BTN_H: i32 = 32;

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
    pub header: Rect,
    pub tasks_tab: Rect,
    pub activity_tab: Rect,
    pub details_toggle: Rect,
    pub logs_toggle: Rect,
    pub group: Rect,
    pub label: Rect,
    pub cells: Vec<Cell>,
    pub details: Option<Rect>,
    pub activity: Option<Rect>,
    pub footer: Rect,
    pub activity_mode: bool,
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
    let h = px(HEADER_H, dpi) + px(MARGIN_TOP, dpi) + group_height(dpi) + px(FOOTER_MIN_H, dpi);
    (w, h)
}

pub fn min_client_with_activity_drawer(dpi: u32) -> (i32, i32) {
    let (w, h) = min_client(dpi);
    (w, h + px(DRAWER_H + MARGIN_TOP, dpi))
}

/// Default client size: the mockup width, minimum height plus a little room for the logo.
pub fn default_client(dpi: u32) -> (i32, i32) {
    let (mw, mh) = min_client(dpi);
    (mw.max(px(DEFAULT_CLIENT_W, dpi)), mh + px(16, dpi))
}

/// Lays out the grid and FluentGruv workspace. The grid never scrolls;
/// activity text owns the only drawer scrollbar.
pub fn compute(client_w: i32, client_h: i32, dpi: u32) -> Layout {
    compute_workspace(client_w, client_h, dpi, false, false, false)
}

pub fn compute_workspace(
    client_w: i32,
    client_h: i32,
    dpi: u32,
    activity_mode: bool,
    details_open: bool,
    activity_open: bool,
) -> Layout {
    let (min_w, base_min_h) = min_client(dpi);
    let min_h = if activity_open && !activity_mode { base_min_h + px(DRAWER_H + MARGIN_TOP, dpi) } else { base_min_h };
    let cw = client_w.max(min_w);
    let ch = client_h.max(min_h);
    let margin = px(MARGIN_X, dpi);
    let header = Rect::new(0, 0, cw, px(HEADER_H, dpi));
    let nav_y = (header.bottom - px(NAV_BTN_H, dpi)) / 2;
    let tasks_tab = Rect::new(px(180, dpi), nav_y, px(88, dpi), px(NAV_BTN_H, dpi));
    let activity_tab = Rect::new(px(276, dpi), nav_y, px(100, dpi), px(NAV_BTN_H, dpi));
    let logs_toggle = Rect::new(cw - margin - px(80, dpi), nav_y, px(80, dpi), px(NAV_BTN_H, dpi));
    let details_toggle = Rect::new(cw - margin - px(168, dpi), nav_y, px(80, dpi), px(NAV_BTN_H, dpi));
    let footer = Rect::new(0, ch - px(FOOTER_MIN_H, dpi), cw, px(FOOTER_MIN_H, dpi));
    let activity = if activity_open && !activity_mode {
        Some(Rect::new(margin, footer.top - px(DRAWER_H, dpi), cw - 2 * margin, px(DRAWER_H, dpi)))
    } else {
        None
    };
    let content_bottom = activity.map(|r| r.top - px(MARGIN_TOP, dpi)).unwrap_or(footer.top);
    let group_top = header.bottom + px(MARGIN_TOP, dpi);
    let dock_details = details_open && cw >= px(1200, dpi);
    let details_width = if details_open { px(DETAILS_W, dpi) } else { 0 };
    let details_gap = if dock_details { px(COL_GAP, dpi) } else { 0 };
    let group_right = cw - margin - if dock_details { details_width + details_gap } else { 0 };
    let group = Rect::new(margin, group_top, group_right - margin, group_height(dpi));
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
        let span_row = row[0].is_some() && row[1].is_none();
        for (c, slot) in row.iter().enumerate() {
            let Some(task) = slot else { continue };
            let left = if span_row { inner_l } else { inner_l + c as i32 * (col_w + col_gap) };
            let width = if span_row || c == 1 { inner_r - left } else { col_w };
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
    let details = if details_open {
        let left = if dock_details { group.right + details_gap } else { cw - margin - details_width };
        Some(Rect { left, top: group_top, right: left + details_width, bottom: content_bottom })
    } else {
        None
    };
    let activity_view = if activity_mode {
        Some(Rect::new(
            margin,
            group_top,
            cw - 2 * margin - if details_open { details_width + px(COL_GAP, dpi) } else { 0 },
            content_bottom - group_top,
        ))
    } else {
        activity
    };
    Layout {
        header,
        tasks_tab,
        activity_tab,
        details_toggle,
        logs_toggle,
        group,
        label,
        cells,
        details,
        activity: activity_view,
        footer,
        activity_mode,
    }
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
        // Rows and the footer stay fixed-size; blank space grows between the group and footer.
        assert_eq!(big.cells[0].main.height(), small.cells[0].main.height());
        assert_eq!(big.footer.height(), small.footer.height());
    }

    #[test]
    fn lone_bottom_left_task_stretches_across_the_grid() {
        let layout = compute(816, 640, 96);
        let bottom = layout.cells.iter().find(|cell| cell.row == GRID.len() - 1).unwrap();
        let inner_left = layout.group.left + px(GROUP_PAD_X, 96);
        let inner_right = layout.group.right - px(GROUP_PAD_X, 96);

        assert_eq!(bottom.col, 0);
        assert_eq!(bottom.main.left, inner_left);
        assert_eq!(bottom.arrow.unwrap().right, inner_right);
    }

    #[test]
    fn below_minimum_is_clamped() {
        let l = compute(10, 10, 96);
        let (w, _) = min_client(96);
        assert_eq!(l.group.right, w - px(MARGIN_X, 96));
    }

    #[test]
    fn fluent_workspace_keeps_grid_and_drawers_inside_standard_viewports() {
        for (w, h, drawer_open) in [(1024, 700, false), (1280, 900, true), (1440, 900, true), (1920, 1080, true)] {
            let l = compute_workspace(w, h, 96, false, false, drawer_open);
            assert!(l.group.right <= w);
            if let Some(drawer) = l.activity {
                assert!(l.group.bottom + px(MARGIN_TOP, 96) <= drawer.top);
                assert!(drawer.bottom <= l.footer.top);
            }
            assert!(l.footer.bottom <= h);
        }
    }

    #[test]
    fn details_dock_on_wide_windows_and_overlay_on_narrow_windows() {
        let narrow = compute_workspace(1024, 700, 96, false, true, false);
        let narrow_details = narrow.details.unwrap();
        assert!(narrow_details.left < narrow.group.right);
        let wide = compute_workspace(1440, 900, 96, false, true, false);
        let wide_details = wide.details.unwrap();
        assert!(wide_details.left >= wide.group.right);
        assert!(wide_details.right <= 1440 - px(MARGIN_X, 96));
    }

    #[test]
    fn activity_mode_reserves_a_scrollable_workspace_and_docked_details() {
        let l = compute_workspace(1440, 900, 96, true, true, false);
        let activity = l.activity.unwrap();
        let details = l.details.unwrap();
        assert!(activity.right < details.left);
        assert_eq!(activity.top, l.group.top);
        assert!(activity.bottom <= l.footer.top);
    }

    #[test]
    fn default_is_mockup_width() {
        assert_eq!(default_client(96).0, 816);
        assert!(default_client(192).0 >= 1632);
    }
}
