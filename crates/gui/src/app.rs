mod file_area;
mod sidebar;
mod title_bar;
mod window_resize;

use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::{HOME, Location, ViewMode};
use crate::style::WINDOW_RADIUS;

/// 文件管理器主视图，持有导航历史、搜索输入和显示设置。
pub struct App {
    history: Vec<Location>,
    history_index: usize,
    search: Entity<InputState>,
    view_mode: ViewMode,
    show_settings: bool,
}

impl App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(HOME.name));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();

        Self {
            history: vec![HOME],
            history_index: 0,
            search,
            view_mode: ViewMode::List,
            show_settings: false,
        }
    }

    fn location(&self) -> Location {
        self.history[self.history_index]
    }

    fn navigate(&mut self, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        if location == self.location() {
            return;
        }

        // 从历史位置打开新目录时，丢弃原来的前进记录。
        self.history.truncate(self.history_index + 1);
        self.history.push(location);
        self.history_index += 1;
        self.reset_search(window, cx);
    }

    fn reset_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 搜索只过滤当前目录，目录名用占位文字显示。
        let name = self.location().name;
        self.search.update(cx, |search, cx| {
            search.set_value("", window, cx);
            search.set_placeholder(name, window, cx);
        });
        cx.notify();
    }
}

impl Render for App {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let square = window.is_maximized()
            || window.is_fullscreen()
            || matches!(window.window_decorations(), Decorations::Client { tiling } if tiling.is_tiled());
        let radius = if square { px(0.) } else { WINDOW_RADIUS };

        v_flex()
            .relative()
            .size_full()
            .rounded(radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.title_bar(window, radius, cx))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(radius, cx))
                    .child(self.file_area(cx)),
            )
            .when(
                !square
                    && window.is_resizable()
                    && matches!(window.window_decorations(), Decorations::Client { .. }),
                |frame| frame.children(window_resize::handles()),
            )
    }
}
