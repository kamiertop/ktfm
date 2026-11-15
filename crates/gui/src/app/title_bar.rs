use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Icon, h_flex};
use gpui_kit::*;

use crate::app::App;
use crate::components::{icon_button, title_bar, with_tooltip};
use crate::style::{SEARCH_MAX_WIDTH, TITLE_ICON_SIZE};

impl App {
    pub(in crate::app) fn title_bar(
        &self,
        window: &mut Window,
        radius: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let home = self.home().map(ToOwned::to_owned);
        let home_disabled = home.as_deref() == self.location() || home.is_none();
        let default_favorite = self
            .location()
            .is_some_and(|path| self.is_default_favorite(path));
        let starred = default_favorite
            || self
                .location()
                .is_some_and(|path| self.workspace.stars.find(path).is_some());
        let star_label = if default_favorite {
            "默认收藏"
        } else if starred {
            "取消收藏"
        } else {
            "收藏当前目录"
        };
        let navigation = h_flex()
            .w_full()
            .max_w(SEARCH_MAX_WIDTH + px(216.))
            .min_w_0()
            .gap_1()
            .child(with_tooltip(
                icon_button("back", IconName::ArrowLeft, "后退")
                    .disabled(!self.can_go_back())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.go_back(window, cx);
                    })),
                "后退",
            ))
            .child(with_tooltip(
                icon_button("forward", IconName::ArrowRight, "前进")
                    .disabled(!self.can_go_forward())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.go_forward(window, cx);
                    })),
                "前进",
            ))
            .child(with_tooltip(
                icon_button("home", IconName::House, "返回主文件夹")
                    .disabled(home_disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(home) = &home {
                            this.navigate(home.clone(), window, cx);
                        }
                    })),
                "返回主文件夹",
            ))
            .child(with_tooltip(
                icon_button("refresh", IconName::RefreshCw, "刷新")
                    .disabled(self.current_location().is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.load_directory(cx))),
                "刷新",
            ))
            .child(with_tooltip(
                icon_button("favorite-directory", IconName::Star, star_label)
                    .toggled(starred)
                    .disabled(default_favorite || self.location().is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(path) = this.location().map(ToOwned::to_owned) {
                            this.toggle_star(path, cx);
                        }
                    })),
                star_label,
            ))
            .child(self.new_tab_button(cx))
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

        // 两处拖拽区独立记录按下状态，区外点击只清除对应区域的状态。
        let title_drag = window.use_state(cx, |_, _| false);
        let toolbar_drag = window.use_state(cx, |_, _| false);
        title_bar(
            navigation,
            radius,
            cx.listener(|this, _, _, cx| this.open_preferences(cx)),
            window,
            [title_drag, toolbar_drag],
            cx,
        )
    }
}
