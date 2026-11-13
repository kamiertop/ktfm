use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Icon, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::App;
use crate::components::{icon_button, title_bar, with_tooltip};
use crate::model::HOME;
use crate::style::{SEARCH_MAX_WIDTH, TITLE_ICON_SIZE};

impl App {
    pub(in crate::app) fn title_bar(
        &self,
        window: &mut Window,
        radius: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let navigation = h_flex()
            .w_full()
            .max_w(SEARCH_MAX_WIDTH + px(112.))
            .min_w_0()
            .gap_1()
            .child(with_tooltip(
                icon_button("back", IconName::ArrowLeft, "后退")
                    .disabled(self.history_index == 0)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.history_index -= 1;
                        this.reset_search(window, cx);
                    })),
                "后退",
            ))
            .child(with_tooltip(
                icon_button("forward", IconName::ArrowRight, "前进")
                    .disabled(self.history_index + 1 == self.history.len())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.history_index += 1;
                        this.reset_search(window, cx);
                    })),
                "前进",
            ))
            .child(with_tooltip(
                icon_button("home", IconName::House, "返回主文件夹")
                    .disabled(self.location() == HOME)
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(HOME, window, cx))),
                "返回主文件夹",
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(SEARCH_MAX_WIDTH)
                    .ml_2()
                    .child(
                        Input::new(&self.search)
                            .prefix(Icon::new(IconName::Search).size(TITLE_ICON_SIZE))
                            .cleanable(true),
                    ),
            );

        // 两处拖拽区各自记录按下状态，避免区外按下事件清除另一处的状态。
        let title_drag = window.use_state(cx, |_, _| false);
        let toolbar_drag = window.use_state(cx, |_, _| false);
        title_bar(
            navigation,
            radius,
            self.show_settings,
            cx.listener(|this, _, _, cx| {
                this.show_settings = !this.show_settings;
                cx.notify();
            }),
            window,
            title_drag,
            toolbar_drag,
            cx,
        )
    }
}
