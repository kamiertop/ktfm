use std::ops::Range;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use daemon::browse::{FileSort, SortColumn, SortDirection};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::popover::PopoverState;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::table::{
    Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::{ActiveTheme, FocusableExt as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::file_columns::{FileColumn, FileColumnSetting, FileListColumns};
use settings::config::view::FileDateFormat;
use settings::config::workspace::Workspace;
use uuid::Uuid;
use vfs::{Entry, EntryKind};

use super::App;
use super::bookmarks::BookmarkDrag;
use super::browser::Browser;
use super::file_area::{PENDING_CUT_OPACITY, format_size};
use super::file_icon::file_icon;
use super::file_operations::FileClipboard;
use super::io_queue::IoQueue;
use crate::components::{icon_button, truncated_text};
use crate::model::entry_appearance;
use crate::style::file_selection_color;

const HEADER_HEIGHT: Pixels = px(34.);
// 表格底部的空白高度，方便取消选中、右键操作
const LIST_BOTTOM_SPACE: Pixels = px(24.);
const RIGHT_CLICK_WIDTH: Pixels = px(24.);
const SCROLLBAR_WIDTH: Pixels = px(12.);
const SIZE_COLUMN_WIDTH: Pixels = px(92.);
const TYPE_COLUMN_WIDTH: Pixels = px(122.);
const MODIFIED_COLUMN_WIDTH: Pixels = px(152.);
const TABLE_END_PADDING: Pixels = px(12.);

fn column_label(column: FileColumn) -> &'static str {
    match column {
        FileColumn::Name => "名称",
        FileColumn::Size => "大小",
        FileColumn::Kind => "类型",
        FileColumn::Modified => "修改时间",
        FileColumn::Created => "创建时间",
        FileColumn::Accessed => "访问时间",
        FileColumn::Location => "所在文件夹",
        FileColumn::Owner => "所有者",
        FileColumn::Group => "组",
        FileColumn::Permissions => "权限",
    }
}

fn column_key_and_width(column: FileColumn) -> (&'static str, Pixels) {
    match column {
        FileColumn::Name => ("name", px(180.)),
        FileColumn::Size => ("size", SIZE_COLUMN_WIDTH),
        FileColumn::Kind => ("kind", TYPE_COLUMN_WIDTH),
        FileColumn::Modified => ("modified", MODIFIED_COLUMN_WIDTH),
        FileColumn::Created => ("created", MODIFIED_COLUMN_WIDTH),
        FileColumn::Accessed => ("accessed", MODIFIED_COLUMN_WIDTH),
        FileColumn::Location => ("location", px(200.)),
        FileColumn::Owner => ("owner", px(114.)),
        FileColumn::Group => ("group", px(114.)),
        FileColumn::Permissions => ("permissions", px(124.)),
    }
}

fn sort_column(column: FileColumn) -> SortColumn {
    match column {
        FileColumn::Name => SortColumn::Name,
        FileColumn::Size => SortColumn::Size,
        FileColumn::Kind => SortColumn::Kind,
        FileColumn::Modified => SortColumn::Modified,
        FileColumn::Created => SortColumn::Created,
        FileColumn::Accessed => SortColumn::Accessed,
        FileColumn::Location => SortColumn::Location,
        FileColumn::Owner => SortColumn::Owner,
        FileColumn::Group => SortColumn::Group,
        FileColumn::Permissions => SortColumn::Permissions,
    }
}

fn column_definition(setting: FileColumnSetting, width: Pixels, sort: FileSort) -> Column {
    let (key, _) = column_key_and_width(setting.column);
    let direction = sort
        .selection()
        .filter(|(column, _)| *column == sort_column(setting.column))
        .map_or(ColumnSort::Default, |(_, direction)| match direction {
            SortDirection::Ascending => ColumnSort::Ascending,
            SortDirection::Descending => ColumnSort::Descending,
        });
    Column::new(key, column_label(setting.column))
        .width(width)
        .min_width(px(FileColumn::MIN_WIDTH))
        .max_width(px(FileColumn::MAX_WIDTH))
        .sort(direction)
}

fn column_text(
    column: FileColumn,
    entry: &Entry,
    child_count: Option<u64>,
    time_format: FileDateFormat,
) -> SharedString {
    match column {
        FileColumn::Name => entry.name.to_string_lossy().into_owned().into(),
        FileColumn::Size if entry.kind == EntryKind::Directory => {
            child_count.map_or_else(|| "—".into(), |count| format!("{count} 个项目").into())
        }
        FileColumn::Size => entry
            .size
            .map_or_else(|| "—".into(), |size| format_size(size).into()),
        FileColumn::Kind => entry_appearance(entry.kind, &entry.path).1.into(),
        FileColumn::Modified => format_time(entry.modified, time_format),
        FileColumn::Created => format_time(entry.created, time_format),
        FileColumn::Accessed => format_time(entry.accessed, time_format),
        FileColumn::Location => entry.path.parent().map_or_else(
            || "—".into(),
            |directory| directory.to_string_lossy().into_owned().into(),
        ),
        FileColumn::Owner => entry
            .access
            .as_ref()
            .map_or_else(|| "—".into(), |access| access.owner.as_ref().into()),
        FileColumn::Group => entry
            .access
            .as_ref()
            .map_or_else(|| "—".into(), |access| access.group.as_ref().into()),
        FileColumn::Permissions => entry.access.as_ref().map_or_else(
            || "—".into(),
            |access| format_permissions(access.mode).into(),
        ),
    }
}

fn format_permissions(mode: u32) -> String {
    let mut text = String::with_capacity(9);
    for (shift, special, lowercase, uppercase) in [
        (6, 0o4000, 's', 'S'),
        (3, 0o2000, 's', 'S'),
        (0, 0o1000, 't', 'T'),
    ] {
        let bits = mode >> shift;
        text.push(if bits & 4 != 0 { 'r' } else { '-' });
        text.push(if bits & 2 != 0 { 'w' } else { '-' });
        text.push(match (mode & special != 0, bits & 1 != 0) {
            (true, true) => lowercase,
            (true, false) => uppercase,
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    text
}

fn format_time(time: Option<std::time::SystemTime>, format: FileDateFormat) -> SharedString {
    let Some(time) = time else {
        return "—".into();
    };
    let time = DateTime::<Local>::from(time);
    let today = if format == FileDateFormat::Simple {
        Local::now().date_naive()
    } else {
        time.date_naive()
    };
    format_local_date(time.naive_local(), today, format).into()
}

fn format_local_date(time: NaiveDateTime, today: NaiveDate, format: FileDateFormat) -> String {
    if format == FileDateFormat::Detailed {
        return time.format("%Y-%m-%d %H:%M").to_string();
    }

    let days = today.signed_duration_since(time.date()).num_days();
    match days {
        -1 => format!("明天 {}", time.format("%H:%M")),
        0 => format!("今天 {}", time.format("%H:%M")),
        1 => format!("昨天 {}", time.format("%H:%M")),
        ..=-2 => time.format("%Y-%m-%d").to_string(),
        2..=6 => format!("{days} 天前"),
        7..=30 => format!("{} 周前", days / 7),
        31..=364 => format!("{} 个月前", days / 31),
        _ => format!("{} 年前", days / 365),
    }
}

pub(super) type FileTable = TableState<FileTableDelegate>;

pub(super) struct FileTableDelegate {
    owner: WeakEntity<App>,
    pane_id: Uuid,
    viewport_width: Pixels,
    columns: Vec<FileColumnSetting>,
    sort: FileSort,
    icon_size: Pixels,
    empty_message: Option<String>,
    show_source: bool,
    time_format: FileDateFormat,
}

impl FileTableDelegate {
    fn browser<'a>(&self, owner: &'a Entity<App>, cx: &'a gpui_kit::App) -> Option<&'a Browser> {
        owner
            .read(cx)
            .ordered_panes()
            .find(|pane| pane.id == self.pane_id)
            .map(|pane| &pane.browser)
    }

    fn column_width(&self, setting: FileColumnSetting) -> Pixels {
        setting.width.map(px).unwrap_or_else(|| {
            let (_, minimum) = column_key_and_width(setting.column);
            if setting.column != FileColumn::Name {
                return minimum;
            }
            let other_columns: Pixels = self.columns[1..]
                .iter()
                .map(|setting| {
                    setting
                        .width
                        .map_or_else(|| column_key_and_width(setting.column).1, px)
                })
                .sum();
            (self.viewport_width
                - other_columns
                - TABLE_END_PADDING
                - RIGHT_CLICK_WIDTH
                - SCROLLBAR_WIDTH)
                .clamp(minimum, px(FileColumn::MAX_WIDTH))
        })
    }

    fn row<'a>(
        &self,
        owner: &'a Entity<App>,
        row: usize,
        cx: &'a gpui_kit::App,
    ) -> Option<(usize, &'a Entry)> {
        let browser = self.browser(owner, cx)?;
        let index = *browser.visible.get(row)?;
        Some((index, browser.entry(index)?))
    }
}

impl TableDelegate for FileTableDelegate {
    fn columns_count(&self, _: &gpui_kit::App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, cx: &gpui_kit::App) -> usize {
        self.owner
            .upgrade()
            .and_then(|owner| {
                self.browser(&owner, cx)
                    .map(|browser| browser.visible.len())
            })
            .unwrap_or(0)
    }

    fn column(&self, column: usize, _: &gpui_kit::App) -> Column {
        let setting = self.columns[column];
        column_definition(setting, self.column_width(setting), self.sort)
    }

    fn render_th(
        &mut self,
        index: usize,
        _: &mut Window,
        cx: &mut Context<FileTable>,
    ) -> impl IntoElement {
        let column = self.columns[index].column;
        let sorted = self
            .sort
            .selection()
            .is_some_and(|(active, _)| active == sort_column(column));
        let theme = cx.theme();
        div()
            .min_w_0()
            .flex_1()
            .truncate()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if sorted {
                theme.foreground
            } else {
                theme.muted_foreground
            })
            .child(column_label(column))
    }

    fn perform_sort(
        &mut self,
        column: usize,
        direction: ColumnSort,
        window: &mut Window,
        cx: &mut Context<FileTable>,
    ) {
        let owner = self.owner.clone();
        let pane_id = self.pane_id;
        let column = sort_column(self.columns[column].column);
        let sort = match direction {
            ColumnSort::Default => FileSort::default(),
            ColumnSort::Ascending => FileSort::new(column, SortDirection::Ascending),
            ColumnSort::Descending => FileSort::new(column, SortDirection::Descending),
        };
        self.sort = sort;
        window.defer(cx, move |_, cx| {
            let _ = owner.update(cx, |app, cx| {
                app.with_pane(pane_id, |app| app.sort_directory(sort, cx));
            });
        });
    }

    fn visible_rows_changed(
        &mut self,
        _: Range<usize>,
        window: &mut Window,
        cx: &mut Context<FileTable>,
    ) {
        if self.sort.by_size()
            || !self
                .columns
                .iter()
                .any(|column| column.column == FileColumn::Size)
        {
            return;
        }
        let owner = self.owner.clone();
        window.defer(cx, move |_, cx| {
            let _ = owner.update(cx, |_, cx| cx.notify());
        });
    }

    fn render_tr(
        &mut self,
        row: usize,
        _: &mut Window,
        cx: &mut Context<FileTable>,
    ) -> Stateful<Div> {
        let empty_row = || div().id(("file-table-row", row));
        let Some(owner) = self.owner.upgrade() else {
            return empty_row();
        };
        let Some(browser) = self.browser(&owner, cx) else {
            return empty_row();
        };
        let Some((index, entry)) = self.row(&owner, row, cx) else {
            return empty_row();
        };
        let revision = browser.snapshot_revision;
        let pane_id = self.pane_id;
        let selected = browser.is_selected(&entry.path);
        let theme = cx.theme();
        let selection = file_selection_color(theme.mode);
        let folder =
            (entry.kind == EntryKind::Directory).then(|| BookmarkDrag::Folder(entry.path.clone()));
        let click_owner = self.owner.clone();
        let middle_owner = self.owner.clone();
        let menu_owner = self.owner.clone();

        div()
            .id(SharedString::from(format!("file-table-row-{revision}-{index}")))
            // 表身保持无横线，表头的列宽手柄使用主题分隔线。
            .border_color(transparent_black())
            .relative()
            .child(
                div().absolute().inset_0().context_menu(move |menu, window, cx| {
                    App::build_file_item_menu(pane_id, revision, index, menu, &menu_owner, window, cx)
                }),
            )
            // 选中背景绘制在组件的悬停背景上方、单元格文字下方。
            .when(selected, |row| {
                row.child(div().absolute().inset_0().bg(selection))
            })
            .when_some(folder, |row, folder| row.on_drag(folder, BookmarkDrag::preview))
            .cursor_pointer()
            .on_click(move |event, window, cx| {
                cx.stop_propagation();
                let _ = click_owner.update(cx, |app, cx| {
                    if app.pane().id == pane_id && app.pane().browser.snapshot_revision == revision {
                        window.focus(&app.focus, cx);
                        app.activate_entry(index, event.click_count(), event.modifiers(), window, cx);
                    }
                });
            })
            .when(entry.kind == EntryKind::Directory, |row| {
                row.on_aux_click(move |event, window, cx| {
                    if matches!(event, ClickEvent::Mouse(click) if click.up.button == MouseButton::Middle) {
                        cx.stop_propagation();
                        let _ = middle_owner.update(cx, |app, cx| {
                            if app.pane().id == pane_id && app.pane().browser.snapshot_revision == revision {
                                app.middle_click_folder(index, window, cx);
                            }
                        });
                    }
                })
            })
    }

    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<FileTable>,
    ) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let Some((index, entry)) = self.row(&owner, row, cx) else {
            return div().into_any_element();
        };
        let column = self.columns[column].column;
        let count = self
            .browser(&owner, cx)
            .and_then(|browser| browser.child_count(index));
        let text = column_text(column, entry, count, self.time_format);
        let cut = cx
            .global::<FileClipboard>()
            .pending_cut_paths()
            .contains(&entry.path);
        let path = entry.path.clone();
        let kind = entry.kind;
        // 占满单元格的内容区，让图标和文字共用行的垂直中心。
        let cell = h_flex()
            .size_full()
            .items_center()
            .min_w_0()
            .when(cut, |cell| cell.opacity(PENDING_CUT_OPACITY));
        if matches!(column, FileColumn::Name) {
            let icon = file_icon(
                kind,
                &path,
                entry_appearance(kind, &path).0,
                self.icon_size,
                window,
                cx,
            );
            let name = v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(truncated_text(text.clone(), text)),
                )
                .when_some(
                    self.show_source.then(|| path.parent()).flatten(),
                    |name, source| {
                        name.child(
                            div()
                                .text_xs()
                                .truncate()
                                .text_color(cx.theme().muted_foreground)
                                .child(source.to_string_lossy().into_owned()),
                        )
                    },
                );
            cell.gap_2()
                .child(h_flex().flex_shrink_0().child(icon))
                .child(name)
                .into_any_element()
        } else {
            cell.child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(cx.theme().muted_foreground)
                    .child(truncated_text(text.clone(), text)),
            )
            .into_any_element()
        }
    }

    fn cell_label(&self, row: usize, column: usize, cx: &gpui_kit::App) -> Option<SharedString> {
        let owner = self.owner.upgrade()?;
        let (index, entry) = self.row(&owner, row, cx)?;
        Some(column_text(
            self.columns[column].column,
            entry,
            self.browser(&owner, cx)?.child_count(index),
            self.time_format,
        ))
    }

    fn render_last_empty_col(
        &mut self,
        _: &mut Window,
        _: &mut Context<FileTable>,
    ) -> impl IntoElement {
        div().w(TABLE_END_PADDING).h_full().flex_shrink_0()
    }

    fn render_empty(&mut self, _: &mut Window, cx: &mut Context<FileTable>) -> impl IntoElement {
        let owner = self.owner.clone();
        let pane_id = self.pane_id;
        v_flex()
            .id("empty-file-table")
            .size_full()
            .justify_center()
            .items_center()
            .text_color(cx.theme().muted_foreground)
            .children(self.empty_message.clone())
            .context_menu(file_list_background_menu(owner, pane_id))
    }
}

fn file_list_background_menu(
    owner: WeakEntity<App>,
    pane_id: Uuid,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu {
    move |menu, _, cx| {
        if let Some(app) = owner.upgrade()
            && app.read(cx).pane().id == pane_id
        {
            return app.read(cx).build_file_background_menu(menu, &owner, cx);
        }
        menu
    }
}

pub(super) struct FileColumnPreferences(pub(super) FileListColumns);

impl Global for FileColumnPreferences {}

fn update_column_preferences(cx: &mut gpui_kit::App, edit: impl FnOnce(&mut FileListColumns)) {
    let mut next = cx.global::<FileColumnPreferences>().0.clone();
    edit(&mut next);
    let next = next.normalize();
    if next == cx.global::<FileColumnPreferences>().0 {
        return;
    }
    cx.set_global(FileColumnPreferences(next.clone()));
    let queued = cx.global::<IoQueue>().enqueue(move || {
        if let Err(error) = Workspace::update(|workspace| workspace.file_list_columns = next) {
            tracing::warn!(%error, "无法保存文件列表列设置");
        }
    });
    if !queued {
        tracing::warn!("存储线程已停止，无法保存文件列表列设置");
    }
}

fn move_column(cx: &mut gpui_kit::App, column: FileColumn, direction: isize) {
    update_column_preferences(cx, |layout| {
        let Some(from) = layout
            .columns
            .iter()
            .position(|setting| setting.column == column)
        else {
            return;
        };
        if let Some(to) = from.checked_add_signed(direction)
            && from > 0
            && to > 0
            && to < layout.columns.len()
        {
            layout.columns.swap(from, to);
        }
    });
}

pub(super) fn file_columns_controls(cx: &mut Context<PopoverState>) -> impl IntoElement + use<> {
    let settings = &cx.global::<FileColumnPreferences>().0.columns;
    let last = settings.len().saturating_sub(1);
    v_flex()
        .min_w(px(230.))
        .pr_5()
        .gap_1()
        .children(settings.iter().enumerate().map(|(index, setting)| {
            let column = setting.column;
            let label = column_label(column);
            h_flex()
                .id(label)
                .gap_2()
                .child(
                    Switch::new("visible")
                        .accessibility_label(label)
                        .checked(setting.visible)
                        .disabled(column == FileColumn::Name)
                        .on_change(cx.listener(move |_, checked: &bool, _, cx| {
                            update_column_preferences(cx, |layout| {
                                if let Some(setting) = layout
                                    .columns
                                    .iter_mut()
                                    .find(|setting| setting.column == column)
                                {
                                    setting.visible = *checked;
                                }
                            });
                            cx.notify();
                        })),
                )
                .child(div().flex_1().child(label))
                .child(
                    icon_button("move-up", IconName::ArrowUp, "上移列")
                        .small()
                        .disabled(index <= 1)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            move_column(cx, column, -1);
                            cx.notify();
                        })),
                )
                .child(
                    icon_button("move-down", IconName::ArrowDown, "下移列")
                        .small()
                        .disabled(index == 0 || index == last)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            move_column(cx, column, 1);
                            cx.notify();
                        })),
                )
        }))
}

impl App {
    pub(super) fn file_list_table(
        &mut self,
        width: Pixels,
        icon_size: Pixels,
        message: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let show_source = matches!(
            self.current_location(),
            Some(settings::config::tab::Location::Tag { .. })
        );
        let columns: Vec<_> = cx
            .global::<FileColumnPreferences>()
            .0
            .columns
            .iter()
            .copied()
            .filter(|column| column.visible)
            .collect();
        let time_format = cx
            .global::<super::preferences::ActivePreferences>()
            .0
            .view
            .file_date_format;
        let pane = &mut self.panes[self.pane_target];
        let table = if let Some(table) = &pane.file_table {
            table.clone()
        } else {
            let table = cx.new(|cx| {
                TableState::new(
                    FileTableDelegate {
                        owner: owner.clone(),
                        pane_id: pane.id,
                        viewport_width: width,
                        columns: columns.clone(),
                        sort: FileSort::default(),
                        icon_size,
                        empty_message: None,
                        show_source,
                        time_format,
                    },
                    window,
                    cx,
                )
                .row_selectable(false)
                .col_selectable(false)
                .col_movable(false)
            });
            cx.subscribe(&table, |_, table, event: &TableEvent, cx| {
                if let TableEvent::ColumnWidthsChanged(widths) = event {
                    let delegate = table.read(cx).delegate();
                    let resized: Vec<_> = delegate
                        .columns
                        .iter()
                        .copied()
                        .zip(widths)
                        .filter(|(column, width)| {
                            (**width - delegate.column_width(*column)).as_f32().abs() >= 0.01
                        })
                        .map(|(column, width)| (column.column, width.as_f32()))
                        .collect();
                    if resized.is_empty() {
                        return;
                    }
                    update_column_preferences(cx, |layout| {
                        for column in &mut layout.columns {
                            if let Some((_, width)) =
                                resized.iter().find(|(id, _)| *id == column.column)
                            {
                                column.width = Some(*width);
                            }
                        }
                    });
                }
            })
            .detach();
            pane.file_table = Some(table.clone());
            table
        };
        let rows = table.read(cx).visible_range().rows().clone();
        let count_visible = columns
            .iter()
            .any(|column| column.column == FileColumn::Size);
        let pane_id = pane.id;
        self.update_child_count_scope(rows, count_visible, cx);
        let pane = &self.panes[self.pane_target];
        let scroll = pane.browser.scroll.clone();
        let row_count = pane.browser.visible.len();
        let show_scrollbar = row_count > 0 && !window.is_inspector_picking(cx);
        let row_height = icon_size + px(24.);
        let rows_end = HEADER_HEIGHT + row_height * row_count;
        table.update(cx, |table, cx| {
            table.vertical_scroll_handle = scroll.clone();
            table.delegate_mut().icon_size = icon_size;
            table.delegate_mut().empty_message = message;
            table.delegate_mut().show_source = show_source;
            table.delegate_mut().time_format = time_format;
            let delegate = table.delegate_mut();
            let refresh = delegate.columns != columns
                || (columns[0].width.is_none() && delegate.viewport_width != width);
            delegate.viewport_width = width;
            if refresh {
                delegate.columns = columns;
                table.refresh(cx);
            }
            cx.notify();
        });

        // 右侧空白属于窗格背景，表格的行和列都止于空白之前。
        let table_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .pr(RIGHT_CLICK_WIDTH + SCROLLBAR_WIDTH)
            .child(
                DataTable::new(&table)
                    .accessibility_label("文件列表")
                    .bordered(false)
                    .focus_ring(false)
                    .header_row_height(HEADER_HEIGHT)
                    .with_size(row_height)
                    .scrollbar_visible(false, true),
            )
            // DataTable 会截获表身的右键，给末行之后的空白接回文件区菜单。
            .when(
                row_count > 0 && rows_end < window.viewport_size().height,
                |area| {
                    area.child(
                        div()
                            .id("file-list-background-below-rows")
                            .absolute()
                            .top(rows_end)
                            .bottom_0()
                            .left_0()
                            .right(RIGHT_CLICK_WIDTH + SCROLLBAR_WIDTH)
                            .context_menu(file_list_background_menu(owner, pane_id)),
                    )
                },
            )
            .when(show_scrollbar, |area| {
                area.child(
                    div()
                        .absolute()
                        .top(HEADER_HEIGHT)
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .child(
                            Scrollbar::vertical(&scroll)
                                .id("file-list-scrollbar")
                                .viewport_from_layout()
                                .styles(|style| style.track(|track| track.width(SCROLLBAR_WIDTH))),
                        ),
                )
            });
        v_flex()
            .size_full()
            .child(table_area)
            .child(div().h(LIST_BOTTOM_SPACE).flex_shrink_0())
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::format_permissions;

    #[test]
    fn date_modes_distinguish_relative_and_full_dates() {
        use chrono::NaiveDate;
        use settings::config::view::FileDateFormat;

        let date = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let today = date.succ_opt().unwrap();
        let timestamp = date.and_hms_opt(8, 30, 0).unwrap();
        assert_eq!(
            super::format_local_date(timestamp, today, FileDateFormat::Simple),
            "昨天 08:30"
        );
        assert_eq!(
            super::format_local_date(timestamp, today, FileDateFormat::Detailed),
            "2026-11-19 08:30"
        );
    }

    #[test]
    fn displays_posix_permissions_with_special_bits() {
        assert_eq!(format_permissions(0o644), "rw-r--r--");
        assert_eq!(format_permissions(0o4755), "rwsr-xr-x");
        assert_eq!(format_permissions(0o2764), "rwxrwSr--");
        assert_eq!(format_permissions(0o1777), "rwxrwxrwt");
    }
}
