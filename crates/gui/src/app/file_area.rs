use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::assets::IconName;

use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::command::AppCommand;
use settings::config::view::ViewMode;
use vfs::EntryKind;

use super::App;
use super::bookmarks::BookmarkDrag;
use super::browser::DirectoryState;
use super::file_operations::FileOperations;
use super::preferences::ActivePreferences;
use super::workspace::WorkspaceWriter;
use crate::components::{icon_button, with_anchored_tooltip};
use crate::model::entry_appearance;
use crate::style::{
    FILE_ICON_SIZE, GRID_ICON_SIZE, GRID_TILE_HEIGHT, GRID_TILE_WIDTH, SIDEBAR_WIDTH,
    file_selection_color, icon_color,
};

const CONTENT_PADDING: Pixels = px(16.);
const FILE_AREA_BOTTOM_PADDING: Pixels = px(56.);
const GRID_GAP: Pixels = px(8.);
const PENDING_CUT_OPACITY: f32 = 0.5;

impl App {
    pub(super) fn file_area(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                if !self.browser.selection.is_empty() {
                    status.push_str(&format!(" · 已选中 {} 项", self.browser.selection.len()));
                }
                if self.file_job_running {
                    status.push_str(" · 正在执行文件操作…");
                }
                if self.browser.filtering {
                    status.push_str(" · 正在筛选…");
                }
                if directory.read_errors > 0 {
                    status.push_str(&format!(" · {} 项读取失败", directory.read_errors));
                }
                (message.map(str::to_owned), status)
            }
        };

        if let Some(row) = self.browser.reveal_row.take() {
            self.browser
                .scroll
                .scroll_to_item(row / columns, ScrollStrategy::Nearest);
        }
        let owner = cx.weak_entity();

        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.path_breadcrumb(cx))
            .child(
                div()
                    .id("file-area-entries")
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.click_file_area_background(event.click_count(), window, cx);
                    }))
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
                                .px(CONTENT_PADDING)
                                .pb(FILE_AREA_BOTTOM_PADDING),
                            )
                            .vertical_scrollbar(&self.browser.scroll)
                    })
                    .map(|area| {
                        if self.location().is_none() && count == 0 {
                            return area.into_any_element();
                        }
                        area.context_menu(move |menu, _, cx| {
                            if let Some(app) = owner.upgrade() {
                                app.read(cx).build_file_background_menu(menu, &owner, cx)
                            } else {
                                menu
                            }
                        })
                        .into_any_element()
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
                        self.browser
                            .action_error
                            .clone()
                            .or_else(|| self.workspace_error.clone())
                            .or_else(|| self.tag_error.clone())
                            .or_else(|| self.tag_state.read(cx).load_error().map(str::to_owned))
                            .or_else(|| cx.global::<WorkspaceWriter>().session_error())
                            .unwrap_or(status),
                    ),
            )
    }

    fn click_file_area_background(
        &mut self,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browser.clear_selection();
        cx.notify();
        if click_count != 2 {
            return;
        }
        let action = cx
            .global::<ActivePreferences>()
            .0
            .actions
            .blank_double_click;
        match action.to_command() {
            Some(AppCommand::NavigateUp) => {
                if let Some(parent) = self.location().and_then(std::path::Path::parent) {
                    self.navigate(parent.to_owned(), window, cx);
                }
            }
            Some(AppCommand::RefreshCurrentDir) => self.load_directory(cx),
            _ => {}
        }
    }

    fn path_to_copy(&self) -> Option<PathBuf> {
        if self.browser.selection.len() == 1 {
            self.browser
                .selection
                .first()
                .and_then(|&index| self.entry_target(index))
                .map(|(path, _)| path)
        } else {
            self.location().map(ToOwned::to_owned)
        }
    }

    pub(super) fn copy_path_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let label = if self.browser.selection.len() == 1 {
            "复制选中文件路径"
        } else {
            "复制当前目录路径"
        };
        with_anchored_tooltip(
            "copy-path",
            icon_button("copy-full-path", IconName::Copy, label)
                .small()
                .disabled(self.browser.selection.len() != 1 && self.location().is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    let Some(path) = this.path_to_copy() else {
                        return;
                    };
                    if let Some(text) = path.to_str() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                        this.browser.action_error = None;
                    } else {
                        this.browser.action_error =
                            Some("无法复制路径：文件名不是有效的 UTF-8 文本".into());
                    }
                    cx.notify();
                })),
            label,
        )
    }

    pub(super) fn hidden_files_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let show_hidden = self.browser.show_hidden;
        let (icon, label) = if show_hidden {
            (IconName::Eye, "隐藏隐藏文件")
        } else {
            (IconName::EyeOff, "显示隐藏文件")
        };
        with_anchored_tooltip(
            "hidden-files",
            icon_button("toggle-hidden-files", icon, label)
                .small()
                .selected(show_hidden)
                .toggled(show_hidden)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_show_hidden_files(!this.browser.show_hidden, cx);
                })),
            label,
        )
    }

    pub(super) fn view_switch_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (mode, icon, label) = match self.view_mode {
            ViewMode::List => (ViewMode::Grid, IconName::LayoutGrid, "切换到网格视图"),
            ViewMode::Grid => (ViewMode::List, IconName::Rows3, "切换到列表视图"),
        };

        with_anchored_tooltip(
            "file-view-mode",
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
        let snapshot_revision = self.browser.snapshot_revision;
        let name = file.name.to_string_lossy().into_owned();
        let (icon, kind) = entry_appearance(file.kind);
        let cut_paths = cx.global::<FileOperations>().pending_cut_paths();
        let target = if file.kind == EntryKind::Directory || !cut_paths.is_empty() {
            self.entry_target(index)
        } else {
            None
        };
        let pending_cut = target
            .as_ref()
            .is_some_and(|(path, _)| cut_paths.contains(path));
        let folder_drag = match target {
            Some((path, EntryKind::Directory)) => Some(BookmarkDrag::Folder(path)),
            _ => None,
        };
        let content_opacity = if pending_cut { PENDING_CUT_OPACITY } else { 1. };
        let owner = cx.weak_entity();
        let selected = self.browser.selection.contains(&index);
        let theme = cx.theme();
        let hover = theme.list_hover;
        let active = file_selection_color(theme.mode);
        let detail_color = if selected {
            theme.foreground
        } else {
            theme.muted_foreground
        };
        Some(
            div()
                .id(SharedString::from(format!(
                    "file-entry-{snapshot_revision}-{index}"
                )))
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
                .when_some(folder_drag, |entry, folder| {
                    entry.on_drag(folder, BookmarkDrag::preview)
                })
                .when(selected, |entry| entry.bg(active))
                .when(!selected, |entry| entry.hover(|style| style.bg(hover)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if this.browser.snapshot_revision == snapshot_revision {
                        window.focus(&this.focus, cx);
                        let modifiers = match event {
                            ClickEvent::Mouse(click) => click.down.modifiers,
                            _ => Modifiers::default(),
                        };
                        this.activate_entry(index, event.click_count(), modifiers, window, cx);
                    }
                }))
                .when(file.kind == EntryKind::Directory, |entry| {
                    entry.on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if matches!(event, ClickEvent::Mouse(click) if click.up.button == MouseButton::Middle)
                            && this.browser.snapshot_revision == snapshot_revision
                        {
                            cx.stop_propagation();
                            this.middle_click_folder(index, window, cx);
                        }
                    }))
                })
                .child(
                    Icon::new(icon)
                        .size(if grid { GRID_ICON_SIZE } else { FILE_ICON_SIZE })
                        .flex_shrink_0()
                        .text_color(icon_color(icon, cx.theme().mode).opacity(content_opacity)),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .max_w_full()
                        .opacity(content_opacity)
                        .child(div().truncate().child(name))
                        .when_some(source, |name, source| {
                            name.child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(detail_color)
                                    .child(source),
                            )
                        }),
                )
                .when(!grid, |row| {
                    row.child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(detail_color.opacity(content_opacity))
                            .child(kind),
                    )
                })
                .context_menu(move |menu, window, cx| {
                    Self::build_file_item_menu(snapshot_revision, index, menu, &owner, window, cx)
                }),
        )
    }
}
