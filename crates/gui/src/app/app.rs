use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::window_resize;
use crate::model::{HOME, Location, ViewMode};
use crate::style::WINDOW_RADIUS;

/// 文件管理器主视图，持有导航历史、搜索输入和显示设置。
pub struct App {
    pub(in crate::app) history: Vec<Location>,
    pub(in crate::app) history_index: usize,
    pub(in crate::app) search: Entity<InputState>,
    pub(in crate::app) view_mode: ViewMode,
    pub(in crate::app) show_settings: bool,
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

    pub(in crate::app) fn location(&self) -> Location {
        self.history[self.history_index]
    }

    pub(in crate::app) fn navigate(
        &mut self,
        location: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if location == self.location() {
            return;
        }

        // 从历史位置打开新目录时，丢弃原来的前进记录。
        self.history.truncate(self.history_index + 1);
        self.history.push(location);
        self.history_index += 1;
        self.reset_search(window, cx);
    }

    pub(in crate::app) fn reset_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // 窗口最大化或全屏时，圆角为 0，避免与系统边框冲突
        let radius = if square { px(0.) } else { WINDOW_RADIUS };
        let border = if square { px(0.) } else { px(1.) };
        v_flex()
            .relative()
            .size_full()
            .rounded(radius)
            .border(border)
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
