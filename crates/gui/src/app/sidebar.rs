use gpui_kit::base::StyledExt;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::SIDEBAR;
use crate::style::{SIDEBAR_ICON_SIZE, SIDEBAR_WIDTH, icon_color, sidebar_divider_color};

use super::App;

impl App {
    pub(super) fn sidebar(&self, radius: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let location = self.location();
        let theme = cx.theme();
        let divider = sidebar_divider_color(theme.mode);
        let (background, border, text, active, hover) = (
            theme.sidebar,
            theme.sidebar_border,
            theme.sidebar_foreground,
            theme.sidebar_accent,
            theme.list_hover,
        );

        let content = v_flex()
            .size_full()
            .px_2()
            .py_3()
            .children(SIDEBAR.iter().enumerate().map(|(index, group)| {
                v_flex()
                    .gap_0p5()
                    .when(index > 0, |group| {
                        group.child(div().mx_2().my_1().h(px(1.)).bg(divider))
                    })
                    .children(group.iter().copied().map(|item| {
                        h_flex()
                            .id(item.name)
                            .h_8()
                            .px_1p5()
                            .gap_2()
                            .rounded_md()
                            .text_base()
                            .cursor_pointer()
                            .when(item == location, |row| row.bg(active).font_semibold())
                            .when(item != location, |row| row.hover(|style| style.bg(hover)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(item, window, cx)
                            }))
                            .child(
                                Icon::new(item.icon)
                                    .size(SIDEBAR_ICON_SIZE)
                                    .text_color(icon_color(item.icon, cx.theme().mode)),
                            )
                            .child(item.name)
                    }))
            }))
            .overflow_y_scrollbar();

        // 背景绘制在滚动区外，滚动时左下圆角仍与窗口对齐。
        div()
            .w(SIDEBAR_WIDTH)
            .min_h_0()
            .flex_shrink_0()
            .rounded_bl(radius)
            .bg(background)
            .text_color(text)
            .child(
                div()
                    .size_full()
                    .border_r_1()
                    .border_color(border)
                    .child(content),
            )
    }
}
