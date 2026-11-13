use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::components::icon_button;
use crate::model::{FileEntry, ViewMode, entries};
use crate::style::{FILE_ICON_SIZE, GRID_ICON_SIZE, GRID_TILE_HEIGHT, GRID_TILE_WIDTH, icon_color};

use super::App;

impl App {
    pub(super) fn file_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let location = self.location();
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut files = entries(location);
        files.retain(|file| file.name.to_lowercase().contains(&query));
        let count = files.len();
        let grid = self.view_mode == ViewMode::Grid;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;

        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .when(self.show_settings, |area| {
                area.child(
                    h_flex()
                        .px_5()
                        .py_2()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .child(div().text_base().mr_2().child("显示方式"))
                        .children(
                            [
                                (ViewMode::List, IconName::List, "列表"),
                                (ViewMode::Grid, IconName::LayoutGrid, "网格"),
                            ]
                            .into_iter()
                            .map(|(mode, icon, label)| {
                                icon_button(label, icon, label)
                                    .label(label)
                                    .toggled(self.view_mode == mode)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.view_mode = mode;
                                        cx.notify();
                                    }))
                            }),
                        ),
                )
            })
            .when(count > 0, |area| {
                let content = div().id("file-content").flex_1().min_h_0().p_4().flex();
                let content = if grid {
                    content.flex_wrap().gap_2().content_start()
                } else {
                    content.flex_col().gap_1()
                };
                area.child(
                    content
                        .children(
                            files
                                .into_iter()
                                .map(|file| self.file_entry(file, grid, cx)),
                        )
                        .overflow_y_scrollbar(),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px_5()
                        .py_2()
                        .border_t_1()
                        .border_color(border)
                        .text_sm()
                        .text_color(muted)
                        .child(format!("{count} 个项目")),
                )
            })
    }

    fn file_entry(
        &self,
        file: FileEntry,
        grid: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let hover = cx.theme().list_hover;
        let muted = cx.theme().muted_foreground;
        div()
            .id(file.name)
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .px_3()
            .text_base()
            .when(grid, |row| {
                row.flex_col()
                    .justify_center()
                    .w(GRID_TILE_WIDTH)
                    .h(GRID_TILE_HEIGHT)
                    .flex_shrink_0()
            })
            .when(!grid, |row| row.h_10().flex_shrink_0())
            .hover(|style| style.bg(hover))
            .when_some(file.destination, |row, destination| {
                row.cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(destination, window, cx);
                    }))
            })
            .child(
                Icon::new(file.icon)
                    .size(if grid { GRID_ICON_SIZE } else { FILE_ICON_SIZE })
                    .text_color(icon_color(file.icon, cx.theme().mode)),
            )
            .child(div().min_w_0().truncate().child(file.name))
            .when(!grid, |row| {
                row.child(div().ml_auto().text_sm().text_color(muted).child(file.kind))
            })
    }
}
