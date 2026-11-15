use std::path::Path;

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon, StyledExt as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::tab::Location;

use crate::app::App;
use crate::components::truncated_text;
use crate::style::icon_color;

impl App {
    pub(super) fn home(&self) -> Option<&Path> {
        self.default_favorites.first()?.path.as_deref()
    }

    pub(super) fn path_appearance(&self, path: &Path) -> (String, IconName) {
        if let Some(item) = self
            .default_favorites
            .iter()
            .find(|item| item.path.as_deref() == Some(path))
        {
            return (item.name.to_owned(), item.icon);
        }
        (
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            IconName::Folder,
        )
    }

    pub(super) fn path_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.location();
        let mut path: Vec<_> = current
            .into_iter()
            .flat_map(Path::ancestors)
            .map(|ancestor| {
                let (name, icon) = self.path_appearance(ancestor);
                (Some(ancestor.to_path_buf()), name, icon)
            })
            .collect();
        if let Some(Location::Tag { .. }) = self.current_location() {
            let (name, icon) = self.location_appearance(self.current_location());
            path.push((None, name, icon));
        }
        let theme = cx.theme();
        let mode = theme.mode;
        let hover_foreground = rgb(if mode.is_dark() { 0x121923 } else { 0xffffff });
        let muted = theme.muted_foreground;
        let border = theme.border;

        let segments =
            path.into_iter()
                .rev()
                .enumerate()
                .map(|(index, (destination, name, icon))| {
                    let active = destination.as_deref() == current;
                    let is_root = destination
                        .as_deref()
                        .is_some_and(|path| path.has_root() && path.parent().is_none());
                    let color = icon_color(icon, mode);
                    let name: SharedString = name.into();
                    let segment = h_flex()
                        .id(("path-segment", index))
                        .min_w_0()
                        .when(is_root, |segment| segment.flex_shrink_0())
                        .h_7()
                        .gap_1()
                        .px_2()
                        .rounded_md()
                        .bg(color.opacity(if active { 0.24 } else { 0.12 }))
                        .text_color(color)
                        .text_sm()
                        .when(active, |segment| segment.font_semibold())
                        .when_some(destination.filter(|_| !active), |segment, destination| {
                            segment
                                .role(Role::Link)
                                .cursor_pointer()
                                .hover(move |style| style.bg(color).text_color(hover_foreground))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.navigate(destination.clone(), window, cx);
                                }))
                        })
                        .child(Icon::new(icon).size(px(15.)).flex_shrink_0())
                        .when(is_root, |segment| segment.child(name.clone()))
                        .when(!is_root, |segment| {
                            segment.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(truncated_text(name.clone(), name)),
                            )
                        });

                    h_flex()
                        .min_w_0()
                        .when(is_root, |item| item.flex_shrink_0())
                        .gap_0p5() // 右侧间距
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
            .when(self.show_tab_bar(), |bar| bar.pt_1())
            .gap_0p5() // 左侧间距
            .border_b_1()
            .border_color(border)
            .children(segments)
    }
}
