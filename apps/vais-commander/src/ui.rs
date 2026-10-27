//! Tiny layout helpers so the rest of the code reads like a stylesheet.

use gpui_kit::*;

pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}
