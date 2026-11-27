use gpui_kit::assets::IconName;
use gpui_kit::base::{InteractiveElementExt as _, StyledExt};
use gpui_kit::component::{ActiveTheme, Icon, Theme, ThemeMode, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::window_controls::window_controls;
use super::{icon_button, with_tooltip};
use crate::style::{
    SIDEBAR_WIDTH, TITLE_BAR_HEIGHT, TITLE_ICON_SIZE, WINDOW_RADIUS, icon_color,
    preferences_palette,
};

pub fn title_bar(
    navigation: impl IntoElement,
    split_controls: impl IntoElement,
    radius: Pixels,
    toggle_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    window: &Window,
    [title_drag, toolbar_drag]: [Entity<bool>; 2],
    cx: &App,
) -> impl IntoElement {
    let mode = cx.theme().mode;
    let (next_mode, theme_icon, theme_label) = match mode {
        ThemeMode::Light => (ThemeMode::Dark, IconName::Moon, "切换为深色"),
        ThemeMode::Dark => (ThemeMode::Light, IconName::Sun, "切换为浅色"),
    };
    let client = matches!(window.window_decorations(), Decorations::Client { .. });

    let theme = cx.theme();
    h_flex()
        .h(TITLE_BAR_HEIGHT)
        .flex_shrink_0()
        .child(drag_region(
            h_flex()
                .w(SIDEBAR_WIDTH)
                .h_full()
                .flex_shrink_0()
                .px(WINDOW_RADIUS)
                .gap_2()
                .rounded_tl(radius)
                .bg(theme.sidebar)
                .border_r_1()
                .border_color(theme.sidebar_border)
                .text_base()
                .font_semibold()
                .child(
                    Icon::new(IconName::Folder)
                        .size(TITLE_ICON_SIZE)
                        .text_color(icon_color(IconName::Folder, mode)),
                )
                .child("ktfm"),
            "title-drag",
            window,
            &title_drag,
        ))
        .child(
            h_flex()
                .h_full()
                .flex_1()
                .min_w_0()
                .pl_1()
                .pr_2()
                .gap_2()
                .child(navigation)
                .child(drag_region(
                    div().h_full().flex_1().min_w(px(24.)),
                    "toolbar-drag",
                    window,
                    &toolbar_drag,
                ))
                .child(h_flex().flex_shrink_0().child(split_controls))
                .child(with_tooltip(
                    icon_button("theme", theme_icon, theme_label).on_click(move |_, window, cx| {
                        Theme::change(next_mode, Some(window), cx);
                    }),
                    theme_label,
                ))
                .child(with_tooltip(
                    icon_button("settings", IconName::Settings, "设置").on_click(toggle_settings),
                    "设置 (Ctrl+P)",
                ))
                .when(client, |bar| bar.child(window_controls(window, cx))),
        )
}

pub fn preferences_title_bar(
    radius: Pixels,
    sidebar_width: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let drag = window.use_state(cx, |_, _| false);
    let theme = cx.theme();
    let palette = preferences_palette(theme.mode);
    let client = matches!(window.window_decorations(), Decorations::Client { .. });
    h_flex()
        .h(TITLE_BAR_HEIGHT)
        .flex_shrink_0()
        .pr_2()
        .child(drag_region(
            h_flex().h_full().flex_1().min_w_0().child(
                h_flex()
                    .w(sidebar_width)
                    .h_full()
                    .flex_shrink_0()
                    .px(WINDOW_RADIUS)
                    .gap_2()
                    .rounded_tl(radius)
                    .bg(palette.sidebar)
                    .border_r_1()
                    .border_color(palette.border)
                    .font_semibold()
                    .child(
                        Icon::new(IconName::Settings)
                            .size(TITLE_ICON_SIZE)
                            .text_color(theme.sidebar_foreground),
                    )
                    .child("偏好设置"),
            ),
            "preferences-title-drag",
            window,
            &drag,
        ))
        .when(client, |bar| bar.child(window_controls(window, cx)))
}

fn drag_region(
    region: Div,
    id: &'static str,
    window: &Window,
    drag: &Entity<bool>,
) -> Stateful<Div> {
    region.id(id).when(
        matches!(window.window_decorations(), Decorations::Client { .. }),
        |region| {
            region
                .on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(drag, |drag, _, _, _| *drag = true),
                )
                .on_mouse_down_out(window.listener_for(drag, |drag, _, _, _| *drag = false))
                .on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(drag, |drag, _, _, _| *drag = false),
                )
                .on_mouse_move(window.listener_for(
                    drag,
                    |drag, event: &MouseMoveEvent, window, _| {
                        if std::mem::take(drag) && event.pressed_button == Some(MouseButton::Left) {
                            window.start_window_move();
                        }
                    },
                ))
                .on_double_click(|_, window, _| {
                    if window.window_controls().maximize {
                        window.zoom_window();
                    }
                })
                .on_mouse_down(MouseButton::Right, |event, window, _| {
                    window.show_window_menu(event.position)
                })
        },
    )
}
