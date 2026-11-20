use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use daemon::job::JobKind;
use gpui_kit::assets::IconName;

use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::command::AppCommand;
use settings::config::view::ViewMode;
use vfs::EntryKind;

use super::App;
use super::bookmarks::BookmarkDrag;
use super::browser::{DirectoryState, SelectedSize};
use super::file_icon::file_icon;
use super::file_operations::FileClipboard;
use super::panes::{clamp_zoom_step, zoom_icon_size};
use super::preferences::ActivePreferences;
use super::workspace::WorkspaceWriter;
use crate::components::{icon_button, truncated_text, with_anchored_tooltip};
use crate::model::entry_appearance;
use crate::style::file_selection_color;

const CONTENT_PADDING: Pixels = px(16.);
const FILE_AREA_BOTTOM_PADDING: Pixels = px(16.);
const GRID_MARGIN: Pixels = px(18.);
// 网格之间的距离
const GRID_GAP: Pixels = px(18.);
const GRID_ICON_SIDE_SPACE: Pixels = px(18.);
const GRID_CARD_PADDING: Pixels = px(6.);
const GRID_CARD_RADIUS: Pixels = px(12.);
const GRID_LABEL_GAP: Pixels = px(6.);
const PENDING_CUT_OPACITY: f32 = 0.5;
const ZOOM_SCROLL_STEP: Pixels = px(48.);
const ZOOM_SCROLL_INTERVAL: Duration = Duration::from_millis(100);

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.;
    let mut unit = "KiB";
    for next_unit in ["MiB", "GiB", "TiB", "PiB", "EiB"] {
        if value < 1024. {
            break;
        }
        value /= 1024.;
        unit = next_unit;
    }
    format!("{value:.1} {unit}")
}

#[derive(Clone, Copy)]
struct FileLayout {
    grid: bool,
    icon_size: Pixels,
    tile_width: Pixels,
    slot_width: Pixels,
    columns: usize,
}

impl FileLayout {
    fn new(mode: ViewMode, step: i8, width: Pixels) -> Self {
        let grid = mode == ViewMode::Grid;
        let mut icon_size = zoom_icon_size(mode, step);
        let content_width = (width - GRID_MARGIN * 2.).max(px(1.));
        let tile_width =
            (icon_size + (GRID_ICON_SIDE_SPACE + GRID_CARD_PADDING) * 2.).min(content_width);
        let columns = if grid {
            ((content_width + GRID_GAP) / (tile_width + GRID_GAP))
                .floor()
                .max(1.) as usize
        } else {
            1
        };
        if grid {
            icon_size = icon_size.min((tile_width - GRID_CARD_PADDING * 2.).max(px(1.)));
        }
        let slot_width = (content_width - GRID_GAP * (columns - 1)) / columns as f32;
        Self {
            grid,
            icon_size,
            tile_width,
            slot_width,
            columns,
        }
    }

    fn row_height(self) -> Pixels {
        self.icon_size + px(24.)
    }
}

fn wheel_zoom_steps(scroll: &mut Option<(Instant, f32)>, now: Instant, delta: f32) -> i8 {
    let (started, remainder) = scroll
        .filter(|(start, amount)| {
            now.duration_since(*start) < ZOOM_SCROLL_INTERVAL && amount.signum() == delta.signum()
        })
        .unwrap_or((now, 0.));
    // 平台可能把一格滚轮上报为多行，每个事件最多缩放一档。
    let remainder = remainder + delta.clamp(-1., 1.);
    let steps = remainder.trunc() as i8;
    *scroll = (steps == 0).then_some((started, remainder));
    steps
}

impl App {
    pub(super) fn file_zoom_shortcut(
        &mut self,
        steps: i8,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.focus.is_focused(window) && self.pane_changes_allowed() {
            cx.stop_propagation();
            self.zoom_file_view(steps, cx);
        }
    }

    fn zoom_file_view(&mut self, steps: i8, cx: &mut Context<Self>) {
        let pane = &mut self.panes[self.pane_target];
        let step = clamp_zoom_step(pane.view_mode, pane.zoom_step.saturating_add(steps));
        if pane.zoom_step != step {
            if pane.view_mode == ViewMode::List {
                pane.browser.preserve_scroll_anchor();
            }
            pane.zoom_step = step;
            self.save_workspace(cx);
        }
    }

    fn scroll_file_zoom(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        if !self.pane_changes_allowed() {
            return;
        }
        let delta = event.delta.pixel_delta(ZOOM_SCROLL_STEP).y / ZOOM_SCROLL_STEP;
        if delta == 0. || !delta.is_finite() {
            return;
        }
        cx.stop_propagation();
        let scroll = &mut self.panes[self.pane_target].zoom_scroll;
        let steps = wheel_zoom_steps(scroll, Instant::now(), delta);
        if steps != 0 {
            self.zoom_file_view(steps, cx);
        }
    }

    pub(super) fn file_area(
        &mut self,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.refresh_selected_size(cx);
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let pane = &self.panes[self.pane_target];
        let pane_id = pane.id;
        let browser = &pane.browser;
        let count = browser.visible.len();
        let layout = FileLayout::new(pane.view_mode, pane.zoom_step, width);
        let selected = browser.selection_len();
        let selection_details = browser.single_selected_entry().map(|entry| {
            let size = match browser.selected_size() {
                Some(SelectedSize::Ready(bytes)) => format_size(*bytes),
                Some(SelectedSize::Loading) => "正在计算大小…".to_owned(),
                _ => "大小未知".to_owned(),
            };
            (size, entry.name.clone())
        });

        let (message, status) = match &self.panes[self.pane_target].browser.state {
            DirectoryState::Unavailable => (Some("无法定位主文件夹".to_owned()), String::new()),
            DirectoryState::Loading { show_message } => (
                show_message.then(|| "正在读取目录…".to_owned()),
                String::new(),
            ),
            DirectoryState::Failed(error) => {
                (Some(format!("无法读取目录：{error}")), String::new())
            }
            DirectoryState::Ready(snapshot) => {
                let directory = &snapshot.directory;
                let message = if self.panes[self.pane_target].browser.filtering && count == 0 {
                    Some("正在筛选…")
                } else if count == 0 && self.panes[self.pane_target].browser.query.is_empty() {
                    Some("没有可显示的项目")
                } else if count == 0 {
                    Some("没有匹配的项目")
                } else {
                    None
                };
                let mut status = format!("{count} 个项目");
                if selected > 1 {
                    status.push_str(&format!(" · 已选中 {selected} 项"));
                }
                if let Some(job) = self.file_job {
                    status.push_str(match job {
                        JobKind::Create => " · 正在新建…",
                        JobKind::Rename => " · 正在重命名…",
                        JobKind::Transfer { cut: false } => " · 正在复制…",
                        JobKind::Transfer { cut: true } => " · 正在移动…",
                        JobKind::Trash => " · 正在移到回收站…",
                    });
                }
                if self.panes[self.pane_target].browser.filtering {
                    status.push_str(" · 正在筛选…");
                }
                if directory.read_errors > 0 {
                    status.push_str(&format!(" · {} 项读取失败", directory.read_errors));
                }
                (message.map(str::to_owned), status)
            }
        };

        let browser = &mut self.panes[self.pane_target].browser;
        if layout.grid {
            browser.prepare_grid_scroll(layout.columns, layout.tile_width, layout.icon_size);
            if let Some(position) = browser.restore_row.take() {
                browser.grid_scroll.scroll_to(ListOffset {
                    item_ix: position / layout.columns,
                    offset_in_item: px(0.),
                });
            }
            if let Some(position) = browser.reveal_row.take() {
                let item_ix = position / layout.columns;
                if browser.grid_scroll.bounds_for_item(item_ix).is_some() {
                    browser.grid_scroll.scroll_to_reveal_item(item_ix);
                } else {
                    browser.grid_scroll.scroll_to(ListOffset {
                        item_ix,
                        offset_in_item: px(0.),
                    });
                }
            }
        } else {
            browser.scroll_layout = Some((1, layout.row_height()));
            if let Some(row) = browser.restore_row.take() {
                browser
                    .scroll
                    .scroll_to_item_strict(row, ScrollStrategy::Top);
            }
            if let Some(row) = browser.reveal_row.take() {
                browser.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
            }
        }
        let owner = cx.weak_entity();
        let zoom_owner = owner.clone();
        let status_error = self.panes[self.pane_target]
            .browser
            .action_error
            .clone()
            .or_else(|| self.workspace_error.clone())
            .or_else(|| self.tag_error.clone())
            .or_else(|| self.tag_state.read(cx).load_error().map(str::to_owned))
            .or_else(|| cx.global::<WorkspaceWriter>().session_error());
        let status_has_error = status_error.is_some();
        let status_foreground = cx.theme().foreground;
        let status_hover = cx.theme().list_hover;

        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(self.path_breadcrumb(window, cx))
            .child(
                div()
                    .id("file-area-entries")
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if this.pane().id == pane_id {
                            window.focus(&this.focus, cx);
                            this.click_file_area_background(event.click_count(), window, cx);
                        }
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
                    .when(count > 0 && layout.grid, |content| {
                        let scroll = &self.panes[self.pane_target].browser.grid_scroll;
                        content
                            .child(
                                list(
                                    scroll.clone(),
                                    cx.processor(move |this, row, window, cx| {
                                        this.with_pane(pane_id, |this| {
                                            this.file_row(row, layout, window, cx)
                                                .into_any_element()
                                        })
                                        .unwrap_or_else(|| div().into_any_element())
                                    }),
                                )
                                .size_full()
                                .p(GRID_MARGIN),
                            )
                            .vertical_scrollbar(scroll)
                    })
                    .when(count > 0 && !layout.grid, |content| {
                        content
                            .child(
                                uniform_list(
                                    "file-list",
                                    count,
                                    cx.processor(move |this, rows: Range<usize>, window, cx| {
                                        this.with_pane(pane_id, |this| {
                                            rows.map(|row| this.file_row(row, layout, window, cx))
                                                .collect()
                                        })
                                        .unwrap_or_default()
                                    }),
                                )
                                .track_scroll(&self.panes[self.pane_target].browser.scroll)
                                .size_full()
                                .px(CONTENT_PADDING)
                                .pb(FILE_AREA_BOTTOM_PADDING),
                            )
                            .vertical_scrollbar(&self.panes[self.pane_target].browser.scroll)
                    })
                    .child(
                        canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, HitboxBehavior::Normal)
                            },
                            move |_, hitbox, window, _| {
                                // 在列表处理滚动前拦截缩放手势。
                                window.on_mouse_event(
                                    move |event: &ScrollEvent, phase, window, cx| {
                                        if phase == DispatchPhase::Capture
                                            && event.control
                                            && hitbox.should_handle_scroll(window)
                                        {
                                            let _ = zoom_owner.update(cx, |app, cx| {
                                                app.with_pane(pane_id, |app| {
                                                    app.scroll_file_zoom(event, cx)
                                                });
                                            });
                                        }
                                    },
                                );
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .map(|area| {
                        if self.location().is_none() && count == 0 {
                            return area.into_any_element();
                        }
                        area.context_menu(move |menu, _, cx| {
                            if let Some(app) = owner.upgrade()
                                && app.read(cx).pane().id == pane_id
                            {
                                app.read(cx).build_file_background_menu(menu, &owner, cx)
                            } else {
                                menu
                            }
                        })
                        .into_any_element()
                    }),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .min_w_0()
                    .gap_2()
                    .px_5()
                    .py_2()
                    .border_t_1()
                    .border_color(border)
                    .text_sm()
                    .text_color(muted)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(status_error.unwrap_or(status)),
                    )
                    .when_some(
                        selection_details.filter(|_| !status_has_error),
                        |bar, (size, name)| {
                            let display_name = name.to_string_lossy().into_owned();
                            let clipboard_name = name.into_string().ok();
                            let label = if clipboard_name.is_some() {
                                "复制名称"
                            } else {
                                "无法复制名称：文件名不是有效的 UTF-8 文本"
                            };
                            let name = h_flex()
                                .id("copy-selected-name")
                                .min_w_0()
                                .gap_1()
                                .px_1()
                                .rounded_sm()
                                .text_color(status_foreground)
                                .when_some(clipboard_name, |name, text| {
                                    name.role(Role::Button)
                                        .cursor_pointer()
                                        .hover(|style| style.bg(status_hover))
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                text.clone(),
                                            ));
                                        })
                                        .child(
                                            Icon::new(IconName::Copy).size(px(12.)).flex_shrink_0(),
                                        )
                                })
                                .child(div().min_w_0().truncate().child(display_name))
                                .tooltip(move |window, cx| Tooltip::new(label).build(window, cx));
                            bar.child(
                                div()
                                    .flex_shrink_0()
                                    .border_l_1()
                                    .border_color(border)
                                    .pl_2()
                                    .child(size),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .border_l_1()
                                    .border_color(border)
                                    .pl_2()
                                    .child(name),
                            )
                        },
                    ),
            )
    }

    fn click_file_area_background(
        &mut self,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = cx
            .global::<ActivePreferences>()
            .0
            .actions
            .blank_double_click
            .to_command();
        self.panes[self.pane_target].browser.clear_selection();
        cx.notify();
        if click_count != 2 {
            return;
        }
        match command {
            Some(AppCommand::RefreshCurrentDir) => self.refresh_directory(cx),
            Some(AppCommand::NavigateUp) => {
                if let Some(parent) = self.location().and_then(std::path::Path::parent) {
                    self.navigate(parent.to_owned(), window, cx);
                }
            }
            _ => {}
        }
    }

    pub(super) fn copy_path_button(
        &self,
        path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let label = "复制路径";
        with_anchored_tooltip(
            "copy-path",
            icon_button("copy-full-path", IconName::Copy, label)
                .small()
                .disabled(path.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let Some(path) = path.as_deref() else { return };
                    if let Some(text) = path.to_str() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                        this.panes[this.pane_target].browser.action_error = None;
                    } else {
                        this.panes[this.pane_target].browser.action_error =
                            Some("无法复制路径：文件名不是有效的 UTF-8 文本".into());
                    }
                    cx.notify();
                })),
            label,
        )
    }

    pub(super) fn hidden_files_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let show_hidden = self.panes[self.pane_target].browser.show_hidden;
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
                    this.set_show_hidden_files(
                        !this.panes[this.pane_target].browser.show_hidden,
                        cx,
                    );
                })),
            label,
        )
    }

    pub(super) fn view_switch_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (mode, icon, label) = match self.panes[self.pane_target].view_mode {
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
        let pane = &mut self.panes[self.pane_target];
        if pane.view_mode != mode {
            pane.view_mode = mode;
            pane.zoom_step = clamp_zoom_step(mode, pane.zoom_step);
            pane.browser.scroll = UniformListScrollHandle::new();
            pane.browser.reset_grid_scroll();
            self.save_workspace(cx);
        }
    }

    fn file_row(
        &mut self,
        row: usize,
        layout: FileLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let start = row * layout.columns;
        let end = (start + layout.columns).min(self.panes[self.pane_target].browser.visible.len());
        h_flex()
            .id(("file-row", row))
            .w_full()
            .when(layout.grid, |row| {
                row.items_start().gap(GRID_GAP).pb(GRID_GAP)
            })
            .when(!layout.grid, |row| row.h(layout.row_height()))
            .children((start..end).filter_map(|position| {
                let index = self.panes[self.pane_target].browser.visible[position];
                let entry = self.file_entry(index, layout, window, cx)?;
                Some(if layout.grid {
                    // 等宽列槽铺满视口，卡片居中；两侧空白由文件区处理点击。
                    h_flex()
                        .w(layout.slot_width)
                        .flex_shrink_0()
                        .justify_center()
                        .child(entry)
                        .into_any_element()
                } else {
                    entry.into_any_element()
                })
            }))
    }

    fn file_entry(
        &mut self,
        index: usize,
        layout: FileLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let DirectoryState::Ready(directory) = &self.panes[self.pane_target].browser.state else {
            return None;
        };
        let file = directory.directory.entries.get(index)?;
        let FileLayout {
            grid,
            tile_width,
            icon_size,
            ..
        } = layout;
        let source = matches!(
            self.current_location(),
            Some(settings::config::tab::Location::Tag { .. })
        )
        .then(|| file.path.parent())
        .flatten()
        .map(|parent| parent.to_string_lossy().into_owned());
        let pane_id = self.panes[self.pane_target].id;
        let snapshot_revision = self.panes[self.pane_target].browser.snapshot_revision;
        let name = SharedString::from(file.name.to_string_lossy().into_owned());
        let (icon, kind, previewable) = entry_appearance(file.kind, &file.path);
        let cut_paths = cx.global::<FileClipboard>().pending_cut_paths();
        let pending_cut = cut_paths.contains(&file.path);
        let folder_drag =
            (file.kind == EntryKind::Directory).then(|| BookmarkDrag::Folder(file.path.clone()));
        let thumbnail = if grid && previewable {
            let device_size = (f32::from(icon_size) * window.scale_factor()).ceil() as u16;
            self.thumbnails.get_or_request(
                (pane_id, snapshot_revision, index),
                &file.path,
                device_size,
                cx,
            )
        } else {
            None
        };
        let is_directory = file.kind == EntryKind::Directory;
        let content_opacity = if pending_cut { PENDING_CUT_OPACITY } else { 1. };
        let owner = cx.weak_entity();
        let selected = self.panes[self.pane_target].browser.is_selected(&file.path);
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
                .rounded_md()
                .text_base()
                .when(grid, |tile| {
                    tile.flex_col()
                        .gap(GRID_LABEL_GAP)
                        .p(GRID_CARD_PADDING)
                        .rounded(GRID_CARD_RADIUS)
                        .w(tile_width)
                        .flex_shrink_0()
                })
                .when(!grid, |entry| {
                    entry.gap_2().px_3().h(layout.row_height() - px(4.)).w_full().min_w_0()
                })
                .when_some(folder_drag, |entry, folder| {
                    entry.on_drag(folder, BookmarkDrag::preview)
                })
                .when(selected, |entry| entry.bg(active))
                .when(!selected, |entry| entry.hover(|style| style.bg(hover)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if this.pane().id == pane_id
                        && this.pane().browser.snapshot_revision == snapshot_revision {
                        window.focus(&this.focus, cx);
                        let modifiers = match event {
                            ClickEvent::Mouse(click) => click.down.modifiers,
                            _ => Modifiers::default(),
                        };
                        this.activate_entry(index, event.click_count(), modifiers, window, cx);
                    }
                }))
                .when(is_directory, |entry| {
                    entry.on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if matches!(event, ClickEvent::Mouse(click) if click.up.button == MouseButton::Middle)
                            && this.pane().id == pane_id
                            && this.pane().browser.snapshot_revision == snapshot_revision
                        {
                            cx.stop_propagation();
                            this.middle_click_folder(index, window, cx);
                        }
                    }))
                })
                .child({
                    let preview = match thumbnail {
                        Some(image) => img(image)
                            .size(icon_size)
                            .object_fit(ObjectFit::Contain)
                            .into_any_element(),
                        None => file_icon(file.kind, &file.path, icon, icon_size, window, cx),
                    };
                    h_flex()
                        .justify_center()
                        .min_w_0()
                        .flex_shrink_0()
                        .opacity(content_opacity)
                        .when(grid, |slot| slot.w_full().h(icon_size))
                        .child(preview)
                })
                .child(
                    v_flex()
                        .min_w_0()
                        .max_w_full()
                        .when(grid, |label| label.w_full())
                        .opacity(content_opacity)
                        .child(
                            div()
                                .when(grid, |label| {
                                    label.text_center()
                                        .text_sm()
                                        .line_height(px(18.))
                                        .whitespace_normal()
                                        .line_clamp(3)
                                        .text_ellipsis()
                                })
                                .when(!grid, |label| label.truncate())
                                .child(truncated_text(name.clone(), name)),
                        )
                        .when_some(source, |name, source| {
                            name.child(
                                div()
                                    .truncate()
                                    .when(grid, |label| {
                                        label.text_center().line_height(px(14.))
                                    })
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
                    Self::build_file_item_menu(pane_id, snapshot_revision, index, menu, &owner, window, cx)
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{FileLayout, GRID_GAP, GRID_MARGIN, ViewMode, wheel_zoom_steps, zoom_icon_size};
    use gpui_kit::px;
    use std::time::{Duration, Instant};

    #[test]
    fn grid_reflows_at_the_next_icon_size() {
        let width = px(610.);
        let original = FileLayout::new(ViewMode::Grid, 0, width);
        let enlarged = FileLayout::new(ViewMode::Grid, 1, width);
        assert_eq!(original.icon_size, px(96.));
        assert_eq!(enlarged.icon_size, px(168.));
        assert!(enlarged.columns < original.columns);
        assert!(enlarged.tile_width > original.tile_width);
        let used = enlarged.tile_width * enlarged.columns + GRID_GAP * (enlarged.columns - 1);
        assert!(used <= width - GRID_MARGIN * 2.);
    }

    #[test]
    fn grid_spreads_unused_width_without_stretching_cards() {
        let first = FileLayout::new(ViewMode::Grid, 0, px(680.));
        let wider = FileLayout::new(ViewMode::Grid, 0, px(780.));
        assert_eq!(first.columns, wider.columns);
        assert_eq!(first.tile_width, wider.tile_width);
        assert!(wider.slot_width > first.slot_width);
        let used = wider.slot_width * wider.columns + GRID_GAP * (wider.columns - 1);
        assert!((used - (px(780.) - GRID_MARGIN * 2.)).as_f32().abs() < 0.01);
    }

    #[test]
    fn zoom_sizes_clamp_to_each_view_modes_limits() {
        assert_eq!(zoom_icon_size(ViewMode::Grid, i8::MIN), px(48.));
        assert_eq!(zoom_icon_size(ViewMode::Grid, i8::MAX), px(256.));
        assert_eq!(zoom_icon_size(ViewMode::List, i8::MIN), px(16.));
        assert_eq!(zoom_icon_size(ViewMode::List, 0), px(32.));
        assert_eq!(zoom_icon_size(ViewMode::List, i8::MAX), px(64.));
    }

    #[test]
    fn wheel_zoom_accumulates_only_recent_movement_in_one_direction() {
        let start = Instant::now();
        let mut scroll = None;
        assert_eq!(wheel_zoom_steps(&mut scroll, start, 3.), 1);
        assert_eq!(wheel_zoom_steps(&mut scroll, start, 0.75), 0);
        assert_eq!(wheel_zoom_steps(&mut scroll, start, -0.25), 0);
        assert_eq!(wheel_zoom_steps(&mut scroll, start, -0.75), -1);

        assert_eq!(wheel_zoom_steps(&mut scroll, start, 0.75), 0);
        let later = start + Duration::from_millis(100);
        assert_eq!(wheel_zoom_steps(&mut scroll, later, 0.25), 0);
        assert_eq!(wheel_zoom_steps(&mut scroll, later, 0.75), 1);
    }
}
