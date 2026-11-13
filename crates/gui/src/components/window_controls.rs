use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::style::WINDOW_CONTROL_ICON_SIZE;

pub(super) fn window_controls(window: &Window, cx: &App) -> impl IntoElement {
    let supported = window.window_controls();
    let (zoom_icon, zoom_label) = if window.is_maximized() {
        (IconName::Copy, "还原窗口")
    } else {
        (IconName::Square, "最大化")
    };
    let theme = cx.theme();
    let normal = ButtonCustomVariant::new(cx)
        .hover(theme.secondary_hover)
        .active(theme.secondary_active);
    let close = ButtonCustomVariant::new(cx)
        .hover(theme.danger.opacity(0.22))
        .active(theme.danger.opacity(0.35));

    h_flex()
        .flex_shrink_0()
        .gap_1()
        .when(supported.minimize, |row| {
            row.child(
                control("window-minimize", IconName::Minus, "最小化", normal)
                    .on_click(|_, window, _| window.minimize_window()),
            )
        })
        .when(supported.maximize, |row| {
            row.child(
                control("window-zoom", zoom_icon, zoom_label, normal)
                    .on_click(|_, window, _| window.zoom_window()),
            )
        })
        .child(
            control("window-close", IconName::X, "关闭窗口", close)
                .on_click(|_, window, _| window.remove_window()),
        )
}

fn control(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    variant: ButtonCustomVariant,
) -> Button {
    Button::new(id)
        .custom(variant)
        .size_8()
        .child(Icon::new(icon).size(WINDOW_CONTROL_ICON_SIZE))
        .accessibility_label(label)
}
