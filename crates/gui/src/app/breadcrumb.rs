use std::iter::successors;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon, StyledExt as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::App;
use crate::style::icon_color;

impl App {
    pub(super) fn path_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.location();
        let path: Vec<_> = successors(Some(current), |location| location.parent()).collect();
        let theme = cx.theme();
        let mode = theme.mode;
        let hover_foreground = rgb(if mode.is_dark() { 0x121923 } else { 0xffffff });
        let muted = theme.muted_foreground;
        let border = theme.border;

        let segments = path.into_iter().rev().enumerate().map(|(index, location)| {
            let active = location == current;
            let color = icon_color(location.icon, mode);
            let segment = h_flex()
                .id(("path-segment", index))
                .min_w_0()
                .h_7()
                .gap_1p5()
                .px_2()
                .rounded_md()
                .bg(color.opacity(if active { 0.24 } else { 0.12 }))
                .text_color(color)
                .text_sm()
                .when(active, |segment| segment.font_semibold())
                .when(!active, |segment| {
                    segment
                        .role(Role::Link)
                        .cursor_pointer()
                        .hover(move |style| style.bg(color).text_color(hover_foreground))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.navigate(location, window, cx);
                        }))
                })
                .child(Icon::new(location.icon).size(px(15.)).flex_shrink_0())
                .child(div().min_w_0().truncate().child(location.name));

            h_flex()
                .min_w_0()
                .gap_1()
                .when(index > 0, |item| {
                    item.child(
                        Icon::new(IconName::ChevronRight)
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(muted),
                    )
                })
                .child(segment)
        });

        h_flex()
            .flex_shrink_0()
            .overflow_hidden()
            .px_3()
            .pb_1p5()
            .gap_1()
            .border_b_1()
            .border_color(border)
            .children(segments)
    }
}
