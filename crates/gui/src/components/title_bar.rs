use gpui_kit::assets::IconName;
use gpui_kit::base::{InteractiveElementExt as _, StyledExt};
use gpui_kit::component::{ActiveTheme, Icon, Theme, ThemeMode, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{icon_button, window_controls::window_controls, with_tooltip};
use crate::style::{SIDEBAR_WIDTH, TITLE_BAR_HEIGHT, TITLE_ICON_SIZE, WINDOW_RADIUS, icon_color};

pub fn title_bar(
    navigation: impl IntoElement,
    radius: Pixels,
    show_settings: bool,
    toggle_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    window: &Window,
    title_drag: Entity<bool>,
    toolbar_drag: Entity<bool>,
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
                .pl_2()
                .pr(WINDOW_RADIUS)
                .gap_2()
                .child(navigation)
                .child(drag_region(
                    div().h_full().flex_1().min_w(px(24.)),
                    "toolbar-drag",
                    window,
                    &toolbar_drag,
                ))
                .child(with_tooltip(
                    icon_button("theme", theme_icon, theme_label).on_click(move |_, window, cx| {
                        Theme::change(next_mode, Some(window), cx);
                    }),
                    theme_label,
                ))
                .child(with_tooltip(
                    icon_button("settings", IconName::Settings, "视图设置")
                        .toggled(show_settings)
                        .on_click(toggle_settings),
                    "视图设置",
                ))
                .when(client, |bar| bar.child(window_controls(window, cx))),
        )
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
