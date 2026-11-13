use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::style::{WINDOW_RESIZE_CORNER, WINDOW_RESIZE_EDGE};

/// 构造覆盖内容边缘的透明缩放热区，按下后交给窗口系统处理拖拽。
pub(super) fn handles() -> impl Iterator<Item = Div> {
    use CursorStyle::*;
    use ResizeEdge::*;

    let corner = || div().size(WINDOW_RESIZE_CORNER);
    let horizontal = || {
        div()
            .h(WINDOW_RESIZE_EDGE)
            .left(WINDOW_RESIZE_CORNER)
            .right(WINDOW_RESIZE_CORNER)
    };
    let vertical = || {
        div()
            .w(WINDOW_RESIZE_EDGE)
            .top(WINDOW_RESIZE_CORNER)
            .bottom(WINDOW_RESIZE_CORNER)
    };

    [
        (Top, horizontal().top_0(), ResizeUpDown),
        (Bottom, horizontal().bottom_0(), ResizeUpDown),
        (Left, vertical().left_0(), ResizeLeftRight),
        (Right, vertical().right_0(), ResizeLeftRight),
        (TopLeft, corner().top_0().left_0(), ResizeUpLeftDownRight),
        (TopRight, corner().top_0().right_0(), ResizeUpRightDownLeft),
        (
            BottomLeft,
            corner().bottom_0().left_0(),
            ResizeUpRightDownLeft,
        ),
        (
            BottomRight,
            corner().bottom_0().right_0(),
            ResizeUpLeftDownRight,
        ),
    ]
    .into_iter()
    .map(|(edge, handle, cursor)| {
        handle.absolute().occlude().cursor(cursor).on_mouse_down(
            MouseButton::Left,
            move |_, window, cx| {
                window.start_window_resize(edge);
                cx.stop_propagation();
            },
        )
    })
}
