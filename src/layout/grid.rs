//! Renders the panel grid as nested resizable groups and maps sizes to/from fractions.

use std::rc::Rc;

use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel, v_resizable};
use gpui_kit::*;

use super::{Layout, MAX_ROWS};
use crate::panel::FilePanel;

pub type OnResize = Rc<dyn Fn(&Entity<ResizableState>, &mut Window, &mut App)>;

/// One state for the row split plus one per row for its column split.
pub struct GridState {
    pub rows: Entity<ResizableState>,
    pub cols: Vec<Entity<ResizableState>>,
}

impl GridState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            rows: cx.new(|_| ResizableState::default()),
            cols: (0..MAX_ROWS)
                .map(|_| cx.new(|_| ResizableState::default()))
                .collect(),
        }
    }

    /// Current sizes as fractions of their container, or `None` before the first layout.
    pub fn fractions(&self, layout: Layout, cx: &App) -> Option<(Vec<f32>, Vec<Vec<f32>>)> {
        let rows = fractions_of(self.rows.read(cx), layout.rows as usize)?;
        let cols = (0..layout.rows as usize)
            .map(|r| fractions_of(self.cols[r].read(cx), layout.cols as usize))
            .collect::<Option<Vec<_>>>()?;
        Some((rows, cols))
    }

    /// Apply saved fractions. Returns `false` when the containers have not been
    /// laid out yet, so the caller should try again on the next render.
    pub fn apply_fractions(
        &self,
        layout: Layout,
        rows: &[f32],
        cols: &[Vec<f32>],
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let ready = |state: &ResizableState, n: usize| {
            state.container_size() > px(0.) && state.sizes().len() == n
        };
        if !ready(self.rows.read(cx), layout.rows as usize) {
            return false;
        }
        for r in 0..layout.rows as usize {
            if !ready(self.cols[r].read(cx), layout.cols as usize) {
                return false;
            }
        }
        apply_to(&self.rows, rows, window, cx);
        for (r, col_fracs) in cols.iter().enumerate().take(layout.rows as usize) {
            apply_to(&self.cols[r], col_fracs, window, cx);
        }
        true
    }
}

fn fractions_of(state: &ResizableState, n: usize) -> Option<Vec<f32>> {
    let container = state.container_size();
    if container <= px(0.) || state.sizes().len() != n {
        return None;
    }
    Some(
        state
            .sizes()
            .iter()
            .map(|s| (*s / container).clamp(0.01, 1.0))
            .collect(),
    )
}

fn apply_to(state: &Entity<ResizableState>, fractions: &[f32], window: &mut Window, cx: &mut App) {
    let container = state.read(cx).container_size();
    let n = fractions.len();
    // Resizing panel `i` moves the boundary between `i` and `i + 1`, so going
    // left to right settles every panel; the last one takes what is left.
    for (i, frac) in fractions.iter().enumerate().take(n.saturating_sub(1)) {
        let size = container * *frac;
        state.update(cx, |state, cx| state.resize_panel(i, size, window, cx));
    }
}

pub fn render_grid(
    layout: Layout,
    grid: &GridState,
    panels: &[Entity<FilePanel>],
    on_resize: OnResize,
) -> impl IntoElement {
    let mut rows_group = v_resizable("grid-rows").with_state(&grid.rows).on_resize({
        let f = on_resize.clone();
        move |s, w, cx| f(s, w, cx)
    });
    for r in 0..layout.rows as usize {
        let mut cols_group = h_resizable(("grid-row", r))
            .with_state(&grid.cols[r])
            .on_resize({
                let f = on_resize.clone();
                move |s, w, cx| f(s, w, cx)
            });
        for c in 0..layout.cols as usize {
            let ix = layout.index(r, c);
            cols_group = cols_group.child(resizable_panel().child(panels[ix].clone()));
        }
        rows_group = rows_group.child(cols_group);
    }
    rows_group
}
