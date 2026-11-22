use std::path::PathBuf;
use std::time::{Duration, Instant};

use daemon::job::JobKind;
use gpui_kit::assets::IconName;

use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::command::AppCommand;
use settings::config::terminal::TerminalMode;
use settings::config::view::ViewMode;
use vfs::EntryKind;

use super::App;
use super::bookmarks::BookmarkDrag;
use super::browser::{DirectoryState, SelectedSize};
use super::file_icon::file_icon;
use super::file_operations::FileClipboard;
use super::file_table::file_columns_controls;
use super::panes::{clamp_zoom_step, zoom_icon_size};
use super::preferences::ActivePreferences;
use super::workspace::WorkspaceWriter;
use crate::components::{icon_button, truncated_text, with_anchored_tooltip};
use crate::model::entry_appearance;
use crate::style::file_selection_color;

// 网格列宽按窗格宽度计算，背景点击使用卡片之间的空隙。
const GRID_MARGIN: Pixels = px(18.);
// 网格之间的距离
const GRID_GAP: Pixels = px(18.);
const GRID_ICON_SIDE_SPACE: Pixels = px(18.);
const GRID_CARD_PADDING: Pixels = px(6.);
const GRID_CARD_RADIUS: Pixels = px(12.);
const GRID_LABEL_GAP: Pixels = px(6.);
pub(super) const PENDING_CUT_OPACITY: f32 = 0.5;
const ZOOM_SCROLL_STEP: Pixels = px(48.);
const ZOOM_SCROLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn format_size(bytes: u64) -> String {
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
        if !grid {
            return Self {
                grid,
                icon_size,
                tile_width: px(0.),
                slot_width: px(0.),
                columns: 1,
            };
        }
        let content_width = (width - GRID_MARGIN * 2.).max(px(1.));
        let tile_width =
            (icon_size + (GRID_ICON_SIDE_SPACE + GRID_CARD_PADDING) * 2.).min(content_width);
        let columns = ((content_width + GRID_GAP) / (tile_width + GRID_GAP))
            .floor()
            .max(1.) as usize;
        icon_size = icon_size.min((tile_width - GRID_CARD_PADDING * 2.).max(px(1.)));
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

fn next_view_mode(mode: ViewMode) -> (ViewMode, IconName, &'static str) {
    match mode {
        ViewMode::List => (ViewMode::Grid, IconName::Rows3, "切换到网格视图"),
        ViewMode::Grid => (ViewMode::List, IconName::LayoutGrid, "切换到列表视图"),
    }
}

fn hidden_files_control(show_hidden: bool) -> (IconName, &'static str) {
    if show_hidden {
        (IconName::Eye, "隐藏隐藏文件")
    } else {
        (IconName::EyeOff, "显示隐藏文件")
    }
}

impl App {
    pub(super) fn file_zoom_shortcut(
        &mut self,
        steps: i8,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let pane = self.pane();
        let list_focused = pane.view_mode == ViewMode::List
            && pane
                .file_table
                .as_ref()
                .is_some_and(|table| table.read(cx).focus_handle(cx).contains_focused(window, cx));
        if (self.focus.is_focused(window) || list_focused) && self.pane_changes_allowed() {
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
            browser.stop_child_counts();
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
        let mut message = message;
        let file_table = (!layout.grid)
            .then(|| self.file_list_table(width, layout.icon_size, message.take(), window, cx));
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
            .child(self.path_breadcrumb(width, window, cx))
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
                    .children(file_table)
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
                    .pl_2()
                    .pr_5()
                    .py_1()
                    .when(width < px(190.), |bar| bar.pr_2())
                    .when(width < px(85.), |bar| bar.px(px(1.)).gap_0p5())
                    .border_t_1()
                    .border_color(border)
                    .text_sm()
                    .text_color(muted)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap_2()
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
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(text.clone()),
                                                    );
                                                })
                                                .child(
                                                    Icon::new(IconName::Copy)
                                                        .size(px(12.))
                                                        .flex_shrink_0(),
                                                )
                                        })
                                        .child(div().min_w_0().truncate().child(display_name))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(label).build(window, cx)
                                        });
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
                    .child(self.file_status_tools(width, cx)),
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
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let label = "复制路径";
        with_anchored_tooltip(
            "copy-path",
            icon_button("copy-full-path", IconName::Copy, label)
                .small()
                .when(compact, |button| button.with_size(px(20.)))
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

    fn file_status_tools(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pane = self.pane();
        let pane_id = pane.id;
        let list = pane.view_mode == ViewMode::List;
        let compact = width < px(190.);
        let tight = width < px(85.);
        let external = cx.global::<ActivePreferences>().0.terminal.mode == TerminalMode::External;
        let terminal_visible = !external && self.embedded_terminal_visible();
        let terminal_label = if external {
            "打开外部终端"
        } else if terminal_visible {
            "隐藏内嵌终端"
        } else {
            "打开内嵌终端"
        };
        let owner = cx.weak_entity();
        h_flex()
            .flex_shrink_0()
            .gap_1()
            .when(tight, |tools| tools.gap(px(1.)))
            .child(with_anchored_tooltip(
                "file-status-terminal",
                icon_button(
                    "file-status-terminal-trigger",
                    IconName::SquareTerminal,
                    terminal_label,
                )
                .small()
                .when(tight, |button| button.with_size(px(16.)))
                .selected(terminal_visible)
                .toggled(terminal_visible)
                .disabled(external && self.location().is_none())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.with_pane(pane_id, |this| this.toggle_terminal(window, cx));
                })),
                terminal_label,
            ))
            .when(!compact, |tools| {
                tools
                    .child(self.view_switch_button(cx))
                    .child(self.hidden_files_button(cx))
            })
            .child(with_anchored_tooltip(
                "file-status-properties",
                icon_button("file-status-properties-trigger", IconName::Info, "属性")
                    .small()
                    .when(tight, |button| button.with_size(px(16.)))
                    .disabled(
                        pane.browser.single_selected_entry().is_none() && self.location().is_none(),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.with_pane(pane_id, |this| this.open_status_properties(window, cx));
                    })),
                "属性",
            ))
            .when(compact || list, |tools| {
                tools.child(
                    Popover::new("file-status-more")
                        .anchor(Anchor::BottomRight)
                        .trigger(
                            icon_button(
                                "file-status-more-trigger",
                                IconName::Ellipsis,
                                if compact {
                                    "更多文件浏览操作"
                                } else {
                                    "列表列设置"
                                },
                            )
                            .small()
                            .when(tight, |button| button.with_size(px(16.))),
                        )
                        .content(move |_, _, cx| {
                            let app = owner.upgrade();
                            let status_actions = app.and_then(|app| {
                                let app = app.read(cx);
                                let pane = app.panes.iter().find(|pane| pane.id == pane_id)?;
                                Some((pane.view_mode, pane.browser.show_hidden))
                            });
                            let mut panel = v_flex().gap_2().min_w(px(174.));
                            if let Some((mode, show_hidden)) = status_actions.filter(|_| compact) {
                                let pane_owner = owner.clone();
                                let (next, _, label) = next_view_mode(mode);
                                panel = panel.child(
                                    Button::new("switch-view").small().label(label).on_click(
                                        move |_, _, cx| {
                                            let _ = pane_owner.update(cx, |app, cx| {
                                                app.with_pane(pane_id, |app| {
                                                    app.set_view_mode(next, cx)
                                                });
                                            });
                                        },
                                    ),
                                );
                                let pane_owner = owner.clone();
                                panel = panel.child(
                                    Button::new("toggle-hidden")
                                        .small()
                                        .label(hidden_files_control(show_hidden).1)
                                        .on_click(move |_, _, cx| {
                                            let _ = pane_owner.update(cx, |app, cx| {
                                                app.with_pane(pane_id, |app| {
                                                    app.set_show_hidden_files(!show_hidden, cx)
                                                });
                                            });
                                        }),
                                );
                            }
                            panel.when(
                                status_actions.map_or(list, |(mode, _)| mode == ViewMode::List),
                                |panel| {
                                    panel
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("列表列设置"),
                                        )
                                        .child(
                                            div()
                                                .max_h(px(230.))
                                                .child(file_columns_controls(cx))
                                                .overflow_y_scrollbar(),
                                        )
                                },
                            )
                        }),
                )
            })
    }

    fn hidden_files_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let show_hidden = self.panes[self.pane_target].browser.show_hidden;
        let (icon, label) = hidden_files_control(show_hidden);
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

    fn view_switch_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (mode, icon, label) = next_view_mode(self.pane().view_mode);

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
            pane.browser.stop_child_counts();
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
            .items_start()
            .gap(GRID_GAP)
            .pb(GRID_GAP)
            .children((start..end).filter_map(|position| {
                let index = self.panes[self.pane_target].browser.visible[position];
                let entry = self.file_entry(index, layout, window, cx)?;
                // 等宽列槽铺满视口，卡片居中；两侧空白由文件区处理点击。
                Some(
                    h_flex()
                        .w(layout.slot_width)
                        .flex_shrink_0()
                        .justify_center()
                        .child(entry),
                )
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
        let (icon, _, previewable) = entry_appearance(file.kind, &file.path);
        let cut_paths = cx.global::<FileClipboard>().pending_cut_paths();
        let pending_cut = cut_paths.contains(&file.path);
        let folder_drag =
            (file.kind == EntryKind::Directory).then(|| BookmarkDrag::Folder(file.path.clone()));
        let thumbnail = if previewable {
            let device_size = (f32::from(icon_size) * window.scale_factor()).ceil() as u16;
            super::thumbnails::ThumbnailKey::new(file)
                .and_then(|key| self.thumbnails.get_or_request(key, device_size, cx))
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
                .flex_col()
                .gap(GRID_LABEL_GAP)
                .p(GRID_CARD_PADDING)
                .rounded(GRID_CARD_RADIUS)
                .w(tile_width)
                .flex_shrink_0()
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
                        .w_full().h(icon_size)
                        .child(preview)
                })
                .child(
                    v_flex()
                        .min_w_0()
                        .max_w_full()
                        .w_full()
                        .opacity(content_opacity)
                        .child(
                            div()
                                .text_center()
                                .text_sm()
                                .line_height(px(18.))
                                .whitespace_normal()
                                .line_clamp(3)
                                .text_ellipsis()
                                .child(truncated_text(name.clone(), name)),
                        )
                        .when_some(source, |name, source| {
                            name.child(
                                div()
                                    .truncate()
                                    .text_center()
                                    .line_height(px(14.))
                                    .text_xs()
                                    .text_color(detail_color)
                                    .child(source),
                            )
                        }),
                )
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
