use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{tab::Location, view::ViewMode};
use vfs::EntryKind;

use super::App;
use super::browser::DirectoryState;
use crate::components::{icon_button, with_anchored_tooltip};
use crate::model::entry_appearance;
use crate::style::{
    FILE_ICON_SIZE, GRID_ICON_SIZE, GRID_TILE_HEIGHT, GRID_TILE_WIDTH, SIDEBAR_WIDTH, icon_color,
};

const CONTENT_PADDING: Pixels = px(16.);
const GRID_GAP: Pixels = px(8.);

impl App {
    pub(super) fn file_area(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let count = self.browser.visible.len();
        let grid = self.view_mode == ViewMode::Grid;
        let content_width =
            window.viewport_size().width - SIDEBAR_WIDTH - CONTENT_PADDING * 2. - px(2.);
        let columns = if grid {
            ((content_width + GRID_GAP) / (GRID_TILE_WIDTH + GRID_GAP))
                .floor()
                .max(1.) as usize
        } else {
            1
        };

        let (message, status) = match &self.browser.state {
            DirectoryState::Unavailable => (Some("无法定位主文件夹".to_owned()), String::new()),
            DirectoryState::Loading => (Some("正在读取目录…".to_owned()), String::new()),
            DirectoryState::Failed(error) => {
                (Some(format!("无法读取目录：{error}")), String::new())
            }
            DirectoryState::Ready(snapshot) => {
                let directory = &snapshot.directory;
                let message = if self.browser.filtering && count == 0 {
                    Some("正在筛选…")
                } else if count == 0 && self.browser.query.is_empty() {
                    Some("没有可显示的项目")
                } else if count == 0 {
                    Some("没有匹配的项目")
                } else {
                    None
                };
                let mut status = format!("{count} 个项目");
                if self.browser.filtering {
                    status.push_str(" · 正在筛选…");
                }
                if directory.read_errors > 0 {
                    status.push_str(&format!(" · {} 项读取失败", directory.read_errors));
                }
                (message.map(str::to_owned), status)
            }
        };

        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.path_breadcrumb(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when_some(message, |content, message| {
                        content.child(
                            v_flex()
                                .size_full()
                                .items_center()
                                .justify_center()
                                .px_4()
                                .text_color(muted)
                                .child(message),
                        )
                    })
                    .when(count > 0, |content| {
                        content
                            .child(
                                uniform_list(
                                    if grid { "file-grid" } else { "file-list" },
                                    count.div_ceil(columns),
                                    cx.processor(move |this, rows: Range<usize>, _, cx| {
                                        rows.map(|row| this.file_row(row, columns, grid, cx))
                                            .collect()
                                    }),
                                )
                                .track_scroll(&self.browser.scroll)
                                .size_full()
                                .px(CONTENT_PADDING),
                            )
                            .vertical_scrollbar(&self.browser.scroll)
                    }),
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
                    .child(
                        self.tag_error
                            .clone()
                            .or_else(|| self.workspace_error.clone())
                            .or_else(|| self.browser.open_error.clone())
                            .unwrap_or(status),
                    ),
            )
    }

    pub(super) fn view_switch_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (mode, icon, label) = match self.view_mode {
            ViewMode::List => (ViewMode::Grid, IconName::LayoutGrid, "切换到网格视图"),
            ViewMode::Grid => (ViewMode::List, IconName::Rows3, "切换到列表视图"),
        };

        with_anchored_tooltip(
            icon_button("switch-file-view", icon, label)
                .small()
                .on_click(cx.listener(move |this, _, _, cx| this.set_view_mode(mode, cx))),
            label,
        )
    }

    pub(super) fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.view_mode != mode {
            self.view_mode = mode;
            self.browser.scroll = UniformListScrollHandle::new();
            cx.notify();
        }
    }

    fn file_row(
        &self,
        row: usize,
        columns: usize,
        grid: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let start = row * columns;
        let end = (start + columns).min(self.browser.visible.len());
        h_flex()
            .id(("file-row", row))
            .w_full()
            .gap(GRID_GAP)
            .h(if grid {
                GRID_TILE_HEIGHT + GRID_GAP
            } else {
                px(44.)
            })
            .children(
                self.browser
                    .visible
                    .get(start..end)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|&index| self.file_entry(index, grid, cx)),
            )
    }

    fn file_entry(
        &self,
        index: usize,
        grid: bool,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let DirectoryState::Ready(directory) = &self.browser.state else {
            return None;
        };
        let file = directory.directory.entries.get(index)?;
        let source = directory
            .paths
            .as_ref()
            .and_then(|paths| paths.get(index))
            .and_then(|path| path.parent())
            .map(|parent| parent.to_string_lossy().into_owned());
        let entity = cx.entity().downgrade();
        let generation = self.browser.generation;
        let name = file.name.to_string_lossy().into_owned();
        let (icon, kind) = entry_appearance(file.kind);
        let hover = cx.theme().list_hover;
        let muted = cx.theme().muted_foreground;
        Some(
            div()
                .id(("file-entry", index))
                .flex()
                .items_center()
                .gap_2()
                .rounded_md()
                .px_3()
                .text_base()
                .when(grid, |tile| {
                    tile.flex_col()
                        .justify_center()
                        .w(GRID_TILE_WIDTH)
                        .h(GRID_TILE_HEIGHT)
                        .flex_shrink_0()
                })
                .when(!grid, |entry| entry.h_10().w_full().min_w_0())
                .hover(|style| style.bg(hover))
                .when(
                    matches!(
                        file.kind,
                        EntryKind::Directory | EntryKind::File | EntryKind::Symlink
                    ),
                    |entry| {
                        entry.cursor_pointer().on_click(cx.listener(
                            move |this, event: &ClickEvent, window, cx| {
                                if this.browser.generation == generation {
                                    this.activate_entry(index, event.click_count(), window, cx);
                                }
                            },
                        ))
                    },
                )
                .child(
                    Icon::new(icon)
                        .size(if grid { GRID_ICON_SIZE } else { FILE_ICON_SIZE })
                        .flex_shrink_0()
                        .text_color(icon_color(icon, cx.theme().mode)),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .max_w_full()
                        .child(div().truncate().child(name))
                        .when_some(source, |name, source| {
                            name.child(div().truncate().text_xs().text_color(muted).child(source))
                        }),
                )
                .when(!grid, |row| {
                    row.child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(muted)
                            .child(kind),
                    )
                })
                .context_menu(move |menu, _, cx| {
                    let Some(this) = entity.upgrade() else {
                        return menu;
                    };
                    let this = this.read(cx);
                    if this.browser.generation != generation {
                        return menu;
                    }
                    let Some((path, kind)) = this.entry_target(index) else {
                        return menu;
                    };
                    let mut menu = menu;
                    if kind == EntryKind::Directory {
                        let target = Location::directory(path.clone());
                        let app = entity.clone();
                        menu = menu.item(PopupMenuItem::new("在新标签页打开").on_click(
                            move |_, window, cx| {
                                let _ = app.update(cx, |this, cx| {
                                    this.new_tab(Some(target.clone()), window, cx)
                                });
                            },
                        ));
                        if !this.is_default_favorite(&path) {
                            let starred = this.workspace.stars.find(&path).is_some();
                            let app = entity.clone();
                            let target = path.clone();
                            menu = menu.item(
                                PopupMenuItem::new(if starred {
                                    "取消收藏"
                                } else {
                                    "收藏文件夹"
                                })
                                .on_click(move |_, _, cx| {
                                    let _ = app.update(cx, |this, cx| {
                                        this.toggle_star(target.clone(), cx)
                                    });
                                }),
                            );
                        }
                    }
                    for tag in this.tags.items() {
                        let id = tag.id;
                        let label = format!("标签 · {}", tag.name);
                        let app = entity.clone();
                        let target = path.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .checked(tag.contains(&path))
                                .on_click(move |_, _, cx| {
                                    let _ = app.update(cx, |this, cx| {
                                        this.toggle_tag_path(id, target.clone(), cx)
                                    });
                                }),
                        );
                    }
                    let app = entity.clone();
                    menu.item(PopupMenuItem::new("新建标签并添加").on_click(
                        move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                this.edit_tag(None, Some(path.clone()), window, cx)
                            });
                        },
                    ))
                }),
        )
    }
}
