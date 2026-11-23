use settings::config::bookmark::BookmarkKind;
use settings::config::tab::SplitLayout;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Icon, h_flex};
use gpui_kit::*;

use super::preferences::ActivePreferences;
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
        let bookmarked = self
            .location()
            .is_some_and(|path| self.bookmarks.read(cx).find(path).is_some());
        let bookmark_label = if bookmarked {
            "从书签移除"
        } else {
            "将当前目录添加到书签"
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
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_directory(cx))),
                "刷新",
            ))
            .child(with_tooltip(
                icon_button("bookmark-directory", IconName::Bookmark, bookmark_label)
                    .toggled(bookmarked)
                    .disabled(self.location().is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(path) = this.location().map(ToOwned::to_owned) {
                            this.set_bookmark(path, BookmarkKind::Directory, !bookmarked, cx);
                        }
                    })),
                bookmark_label,
            ))
            .child(self.new_tab_button(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(SEARCH_MAX_WIDTH)
                    .ml_2()
                    .child(
                        Input::new(&self.pane().search)
                            .prefix(Icon::new(IconName::Search).size(TITLE_ICON_SIZE))
                            .cleanable(true),
                    ),
            );

        let shortcuts = &cx.global::<ActivePreferences>().0.shortcuts;
        let split_button = |layout, id, icon, open_label: &'static str, shortcut: &str| {
            let can_split = self.can_split(layout);
            let merge_label = match layout {
                SplitLayout::SideBySide if !can_split && self.can_merge_horizontal() => {
                    Some("合并左右分栏")
                }
                SplitLayout::Stacked if self.can_merge_vertical() => Some("合并当前列"),
                _ => None,
            };
            let merge = merge_label.is_some();
            let (label, shortcut) = if let Some(label) = merge_label {
                (label, "")
            } else {
                (open_label, shortcut)
            };
            let tooltip = if shortcut.is_empty() {
                label.to_owned()
            } else {
                format!("{label} ({shortcut})")
            };
            with_tooltip(
                icon_button(id, icon, label)
                    .disabled(!can_split && !merge)
                    .toggled(merge)
                    .on_click(
                        cx.listener(move |this, _, window, cx| match (layout, merge) {
                            (SplitLayout::SideBySide, true) => this.merge_horizontal(window, cx),
                            (SplitLayout::Stacked, true) => this.merge_current_column(window, cx),
                            _ => this.open_split(layout, true, window, cx),
                        }),
                    ),
                tooltip,
            )
        };
        let split_controls = h_flex()
            .child(split_button(
                SplitLayout::SideBySide,
                "split-right",
                IconName::Columns3,
                "左右分栏",
                &shortcuts.split_pane,
            ))
            .child(split_button(
                SplitLayout::Stacked,
                "split-down",
                IconName::Rows2,
                "上下分栏",
                &shortcuts.split_down,
            ));

        // 两处拖拽区独立记录按下状态，区外点击只清除对应区域的状态。
        let title_drag = window.use_state(cx, |_, _| false);
        let toolbar_drag = window.use_state(cx, |_, _| false);
        title_bar(
            navigation,
            split_controls,
            radius,
            cx.listener(|this, _, _, cx| this.open_preferences(cx)),
            window,
            [title_drag, toolbar_drag],
            cx,
        )
    }
}
