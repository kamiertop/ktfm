use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::tab::Location;

use super::file_icon::file_icon;
use crate::app::App;
use crate::components::{icon_button, with_anchored_tooltip};
use crate::model::entry_appearance;
use crate::style::icon_color;

const SCROLLBAR_HEIGHT: Pixels = px(6.);

#[derive(Default)]
pub(super) struct BreadcrumbScroll {
    handle: ScrollHandle,
    navigation_revision: u64,
    selected_path: Option<PathBuf>,
}

fn reveal_path_end(scroll: &ScrollHandle) {
    // GPUI 按当前布局的实际溢出宽度修正偏移。
    scroll.set_offset(point(-Pixels::MAX, px(0.)));
}

impl App {
    pub(super) fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    pub(super) fn path_appearance(&self, path: &Path) -> (String, IconName) {
        (
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            IconName::Folder,
        )
    }

    pub(super) fn path_breadcrumb(
        &mut self,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let copy_path = self
            .pane()
            .browser
            .single_selected_entry()
            .map(|entry| entry.path.as_path())
            .or_else(|| self.location())
            .map(Path::to_owned);
        let editing = self.pane().path_editor.is_some();
        let path = if let Some(editor) = self.path_editor_view(cx) {
            editor.into_any_element()
        } else {
            self.breadcrumb_strip(window, cx).into_any_element()
        };
        let compact = width < px(105.);
        let (edit_icon, edit_label) = if editing {
            (IconName::X, "取消编辑路径")
        } else {
            (IconName::Pencil, "编辑路径")
        };
        h_flex()
            .w_full()
            .flex_shrink_0()
            .min_w_0()
            .px_2()
            .pb(SCROLLBAR_HEIGHT)
            .when(self.show_tab_bar(), |bar| bar.pt_1())
            .gap_1()
            .when(compact, |bar| {
                bar.flex_col().items_stretch().px_1().gap(SCROLLBAR_HEIGHT)
            })
            .when(editing && !compact, |bar| bar.items_start())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(path)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .when(compact, |buttons| buttons.self_end().gap_0p5())
                    .child(with_anchored_tooltip(
                        "edit-path",
                        icon_button("edit-path-trigger", edit_icon, edit_label)
                            .small()
                            .when(compact, |button| button.with_size(px(20.)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_path_editor(window, cx);
                            })),
                        edit_label,
                    ))
                    .child(self.copy_path_button(copy_path, compact, cx)),
            )
    }

    fn breadcrumb_strip(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let pane = &mut self.panes[self.pane_target];
        let selected_path = pane
            .browser
            .single_selected_entry()
            .map(|entry| &entry.path);
        let scroll = &mut pane.breadcrumb_scroll;
        if scroll.navigation_revision != pane.navigation_revision
            || scroll.selected_path.as_ref() != selected_path
        {
            scroll.navigation_revision = pane.navigation_revision;
            scroll.selected_path = selected_path.cloned();
            reveal_path_end(&scroll.handle);
        }
        let scroll = scroll.handle.clone();
        let resize_scroll = scroll.clone();
        let mut path: Vec<_> = self
            .location()
            .into_iter()
            .flat_map(Path::ancestors)
            .map(|ancestor| {
                let (name, icon) = self.path_appearance(ancestor);
                (Some(ancestor.to_path_buf()), name, icon)
            })
            .collect();
        path.reverse();
        if let Some(location @ Location::Tag { .. }) = self.current_location() {
            let (name, icon) = self.location_appearance(Some(location), cx);
            path.push((None, name, icon));
        }
        let selected = self.pane().browser.single_selected_entry();
        if let Some(entry) = selected {
            let (icon, _, _) = entry_appearance(entry.kind, &entry.path);
            path.push((None, entry.name.to_string_lossy().into_owned(), icon));
        }
        let selected_index = selected.map(|_| path.len() - 1);
        let active_index = path.len().checked_sub(1);
        let theme = cx.theme();
        let mode = theme.mode;
        let muted = theme.muted_foreground;

        let segments = path
            .into_iter()
            .enumerate()
            .map(|(index, (destination, name, icon))| {
                let active = active_index == Some(index);
                let color = icon_color(icon, mode);
                let name: SharedString = name.into();
                let icon = if selected_index == Some(index)
                    && let Some(entry) = selected
                {
                    file_icon(entry.kind, &entry.path, icon, px(15.), window, cx)
                } else {
                    Icon::new(icon)
                        .size(px(15.))
                        .flex_shrink_0()
                        .into_any_element()
                };
                let segment = h_flex()
                    .id(("path-segment", index))
                    .flex_shrink_0()
                    .h_7()
                    .gap_1()
                    .px_2()
                    .rounded_md()
                    .bg(color.opacity(if active { 0.24 } else { 0.12 }))
                    .text_color(color)
                    .text_sm()
                    .whitespace_nowrap()
                    .when(active, |segment| segment.font_semibold())
                    .when_some(destination.filter(|_| !active), |segment, destination| {
                        segment
                            .role(Role::Link)
                            .cursor_pointer()
                            // 滚动时排版与绘制的悬停状态可能不同，文字颜色保持一致。
                            .hover(move |style| style.bg(color.opacity(0.28)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(destination.clone(), window, cx);
                            }))
                    })
                    .child(icon)
                    .child(name);

                h_flex()
                    .flex_shrink_0()
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
            .flex_1()
            .min_w_0()
            .max_w_full()
            .relative()
            .on_prepaint(move |bounds, _, _| {
                if bounds.size.width != resize_scroll.bounds().size.width
                    && resize_scroll.offset().x + resize_scroll.max_offset().x <= Pixels::ZERO
                {
                    reveal_path_end(&resize_scroll);
                }
            })
            .child(
                h_flex()
                    .id("breadcrumb-scroll")
                    .w_full()
                    .h_7()
                    .overflow_hidden()
                    .overflow_x_scroll()
                    .track_scroll(&scroll)
                    .gap_0p5() // IconName::ChevronRight 的左侧间距
                    .children(segments),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(relative(1.))
                    .h(SCROLLBAR_HEIGHT)
                    .child(
                        Scrollbar::horizontal(&scroll)
                            .id("breadcrumb-scrollbar")
                            .viewport_from_layout()
                            .mode(ScrollbarMode::Always)
                            .styles(|styles| {
                                styles
                                    .track(|track| track.width(SCROLLBAR_HEIGHT))
                                    .thumb(|thumb| thumb.width(px(2.)).inset(px(2.)))
                                    .thumb_active(|thumb| thumb.width(px(4.)).inset(px(1.)))
                            }),
                    ),
            )
    }
}
