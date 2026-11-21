use gpui_kit::base::ElementExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::resizable::{
    ResizablePanelGroup, h_resizable, resizable_panel, v_resizable,
};
use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    tab::{GridSession, PaneColumn, SplitLayout},
    view::ViewMode,
};
use uuid::Uuid;

use super::App;
use super::breadcrumb::BreadcrumbScroll;
use super::browser::Browser;
use super::embed_terminal::TerminalResize;
use super::file_table::FileTable;
use super::path_editor::PathEditor;
use super::preferences::ActivePreferences;
use super::tabs::{BrowserTab, restore_tab_items};
use super::tags::TagChange;
use crate::style::{SIDEBAR_WIDTH, TITLE_BAR_HEIGHT};

const MAX_COLUMNS: usize = 3;
const MAX_PANES: usize = 6;

const GRID_ZOOM_SIZES: [Pixels; 5] = [px(48.), px(64.), px(96.), px(168.), px(256.)];
const LIST_ZOOM_SIZES: [Pixels; 3] = [px(16.), px(32.), px(64.)];

fn zoom_sizes(mode: ViewMode) -> &'static [Pixels] {
    match mode {
        ViewMode::Grid => &GRID_ZOOM_SIZES,
        ViewMode::List => &LIST_ZOOM_SIZES,
    }
}

pub(super) fn clamp_zoom_step(mode: ViewMode, step: i8) -> i8 {
    let midpoint = (zoom_sizes(mode).len() / 2) as i8;
    step.clamp(-midpoint, midpoint)
}

pub(super) fn zoom_icon_size(mode: ViewMode, step: i8) -> Pixels {
    let sizes = zoom_sizes(mode);
    let index = sizes.len() as i8 / 2 + clamp_zoom_step(mode, step);
    sizes[index as usize]
}

mod layout;
pub(super) use layout::PaneLayout;
const MIN_COLUMN_WIDTH: Pixels = px(200.);
const MIN_ROW_HEIGHT: Pixels = px(140.);

pub(super) struct BrowserPane {
    pub(super) id: Uuid,
    measured_size: Option<Size<Pixels>>,
    pub(super) tabs: Vec<BrowserTab>,
    pub(super) active_tab: usize,
    pub(super) navigation_revision: u64,
    pub(super) search: Entity<InputState>,
    pub(super) browser: Browser,
    pub(super) breadcrumb_scroll: BreadcrumbScroll,
    pub(super) path_editor: Option<PathEditor>,
    pub(super) file_table: Option<Entity<FileTable>>,
    pub(super) view_mode: ViewMode,
    pub(super) zoom_step: i8,
    pub(super) zoom_scroll: Option<(std::time::Instant, f32)>,
}

impl BrowserPane {
    pub(super) fn restore_view(&mut self, mode: Option<ViewMode>, zoom_step: i8) {
        if let Some(mode) = mode {
            self.view_mode = mode;
        }
        self.zoom_step = clamp_zoom_step(self.view_mode, zoom_step);
    }

    pub(super) fn new(
        tabs: Vec<BrowserTab>,
        active_tab: usize,
        window: &mut Window,
        cx: &mut Context<App>,
    ) -> Self {
        let id = Uuid::new_v4();
        let view = &cx.global::<ActivePreferences>().0.view;
        let (view_mode, show_hidden) = (view.view_mode, view.show_hidden_files);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索当前页"));
        cx.observe(&search, move |app, search, cx| {
            let query = search.read(cx).value().trim().to_lowercase();
            app.with_pane(id, |app| app.search_directory(query, cx));
        })
        .detach();
        Self {
            id,
            measured_size: None,
            tabs,
            active_tab,
            navigation_revision: 0,
            search,
            browser: Browser::new(show_hidden),
            breadcrumb_scroll: BreadcrumbScroll::default(),
            path_editor: None,
            file_table: None,
            view_mode,
            zoom_step: 0,
            zoom_scroll: None,
        }
    }
}

#[derive(Clone, Copy)]
enum SplitGroup {
    Columns,
    Rows(usize),
    RowPanes(usize, usize),
}

impl App {
    // 后台操作可以临时指定目标窗格，用户焦点独立保存。
    pub(super) fn with_pane<R>(&mut self, id: Uuid, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let index = self.pane_index(id)?;
        let previous = std::mem::replace(&mut self.pane_target, index);
        let result = f(self);
        self.pane_target = previous;
        Some(result)
    }

    pub(super) fn pane(&self) -> &BrowserPane {
        &self.panes[self.pane_target]
    }

    pub(super) fn ordered_panes(&self) -> impl Iterator<Item = &BrowserPane> {
        self.panes.iter()
    }

    fn pane_index(&self, id: Uuid) -> Option<usize> {
        self.panes.iter().position(|pane| pane.id == id)
    }

    fn select_pane(&mut self, index: usize) {
        self.active_pane = index;
        self.pane_target = index;
    }

    pub(super) fn pane_changes_allowed(&self) -> bool {
        self.file_name_editor.is_none()
            && self.file_properties.is_none()
            && self.tag_editor.is_none()
            && self.bookmark_rename.is_none()
            && self.bookmark_properties.is_none()
            && self.volume_unlock.is_none()
    }

    pub(super) fn can_split(&self, layout: SplitLayout) -> bool {
        self.pane_layout.can_split(self.active_pane, layout)
    }

    pub(super) fn can_merge_vertical(&self) -> bool {
        self.pane_layout.columns[self.pane_layout.pane_column(self.active_pane).0].rows == 2
    }

    pub(super) fn can_merge_horizontal(&self) -> bool {
        let (column, row, _) = self.pane_layout.pane_row(self.active_pane);
        let current = &self.pane_layout.columns[column];
        current.row_panes[row] > 1 || self.pane_layout.columns.len() > 1
    }

    pub(super) fn open_split(
        &mut self,
        layout: SplitLayout,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_changes_allowed() || !self.can_split(layout) {
            return;
        }
        let index = self.pane_layout.split(self.active_pane, layout, forward);
        let tab = BrowserTab::new(Uuid::new_v4(), self.current_location().cloned());
        let pane = BrowserPane::new(vec![tab], 0, window, cx);
        self.panes.insert(index, pane);
        self.select_pane(index);
        self.load_directory(cx);
        window.focus(&self.focus, cx);
        self.save_workspace(cx);
    }

    pub(super) fn restore_split(
        &mut self,
        grid: Option<GridSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = grid else { return };
        let Some(layout) = PaneLayout::restore(grid.columns, grid.panes.len() + 1) else {
            // 布局异常时，仍可在主窗格打开保存的标签。
            for pane in grid.panes {
                let (tabs, _) = restore_tab_items(
                    &pane.tabs,
                    pane.active,
                    self.tag_state.read(cx).loaded_tags(),
                    None,
                );
                self.panes[self.pane_target].tabs.extend(tabs);
            }
            return;
        };
        self.pane_layout = layout;
        for pane in grid.panes {
            let (tabs, selected) = restore_tab_items(
                &pane.tabs,
                pane.active,
                self.tag_state.read(cx).loaded_tags(),
                self.home().map(ToOwned::to_owned),
            );
            let mut restored = BrowserPane::new(tabs, selected, window, cx);
            restored.restore_view(pane.view_mode, pane.zoom_step);
            self.panes.push(restored);
        }
        let inactive: Vec<_> = self
            .panes
            .iter()
            .filter(|pane| pane.id != self.panes[self.active_pane].id)
            .map(|pane| pane.id)
            .collect();
        for id in inactive {
            self.with_pane(id, |app| app.load_directory(cx));
        }
        self.select_pane(grid.active_index.min(self.panes.len() - 1));
    }

    fn focus_pane(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index != self.active_pane && self.pane_changes_allowed() {
            self.select_pane(index);
            window.focus(&self.focus, cx);
            self.save_workspace(cx);
        }
    }

    fn merge_neighbor(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pane_changes_allowed() {
            return;
        }
        let id = self.ordered_panes().nth(index).expect("相邻窗格存在").id;
        self.with_pane(id, |app| app.remember_active_tab(cx));
        let (column, row, _) = self.pane_layout.pane_row(index);
        self.pane_layout.remove_pane(column, row);
        let pane = self.panes.remove(index);
        self.select_pane(self.active_pane - usize::from(index < self.active_pane));
        self.panes[self.pane_target].tabs.extend(pane.tabs);
        window.focus(&self.focus, cx);
        self.save_workspace(cx);
    }

    pub(super) fn merge_current_column(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self
            .neighbor(SplitLayout::Stacked, true)
            .or_else(|| self.neighbor(SplitLayout::Stacked, false))
        {
            self.merge_neighbor(index, window, cx);
        }
    }

    pub(super) fn merge_horizontal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (column, row, start) = self.pane_layout.pane_row(self.active_pane);
        let count = self.pane_layout.columns[column].row_panes[row];
        let index = if count > 1 {
            // 行内有分栏时，先合并同行窗格。
            if self.active_pane + 1 < start + count {
                Some(self.active_pane + 1)
            } else {
                Some(self.active_pane - 1)
            }
        } else {
            self.neighbor(SplitLayout::SideBySide, true)
                .or_else(|| self.neighbor(SplitLayout::SideBySide, false))
        };
        if let Some(index) = index {
            self.merge_neighbor(index, window, cx);
        }
    }

    pub(super) fn close_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pane_changes_allowed() || self.panes.len() == 1 {
            return;
        }
        let inactive: Vec<_> = self
            .panes
            .iter()
            .filter(|pane| pane.id != self.panes[self.active_pane].id)
            .map(|pane| pane.id)
            .collect();
        for id in inactive {
            self.with_pane(id, |app| app.remember_active_tab(cx));
        }
        let mut active = self.panes.remove(self.active_pane);
        for pane in self.panes.drain(..) {
            active.tabs.extend(pane.tabs);
        }
        self.panes.push(active);
        self.select_pane(0);
        self.pane_layout = PaneLayout::default();
        window.focus(&self.focus, cx);
        self.save_workspace(cx);
    }

    pub(super) fn close_empty_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.len() > 1 && self.pane_changes_allowed() {
            let (column, row, start) = self.pane_layout.pane_row(self.active_pane);
            let layout = self.pane_layout.columns[column];
            let index = if layout.row_panes[row] > 1 {
                self.active_pane.min(start + layout.row_panes[row] - 2)
            } else if layout.rows == 2 {
                if row == 0 {
                    start
                } else {
                    start - layout.row_panes[0]
                }
            } else {
                self.active_pane.min(self.panes.len() - 2)
            };
            self.pane_layout.remove_pane(column, row);
            self.panes.remove(self.active_pane);
            self.select_pane(index);
            window.focus(&self.focus, cx);
            self.save_workspace(cx);
        }
    }

    pub(super) fn sync_tags_in_panes(
        &mut self,
        event: &TagChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_tag_change(event, window, cx);
        let inactive: Vec<_> = self
            .panes
            .iter()
            .filter(|pane| pane.id != self.panes[self.active_pane].id)
            .map(|pane| pane.id)
            .collect();
        for id in inactive {
            self.with_pane(id, |app| app.sync_tag_change(event, window, cx));
        }
        self.save_workspace(cx);
    }

    pub(super) fn focus_split_pane(
        &mut self,
        layout: SplitLayout,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.neighbor(layout, forward) {
            self.focus_pane(index, window, cx);
        }
    }

    fn neighbor(&self, layout: SplitLayout, forward: bool) -> Option<usize> {
        self.pane_layout.neighbor(self.active_pane, layout, forward)
    }

    pub(super) fn browser_panes(
        &mut self,
        border_width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport = window.viewport_size();
        let width = (viewport.width - SIDEBAR_WIDTH - border_width * 2.).max(px(0.));
        let height = (viewport.height - TITLE_BAR_HEIGHT - border_width * 2.).max(px(0.));
        if self.panes.len() == 1 {
            return self
                .browser_pane(size(width, height), false, true, window, cx)
                .into_any_element();
        }

        let active = self.panes[self.pane_target].id;
        let ids: Vec<_> = self.ordered_panes().map(|pane| pane.id).collect();
        let columns = self.pane_layout.columns.clone();
        let widths = column_extents(width, &columns);
        let column_minimum = MIN_COLUMN_WIDTH.min(width / columns.len() as f32);
        let structure: Vec<_> = columns
            .iter()
            .map(|column| (column.rows, column.row_panes))
            .collect();
        // 分屏结构变化时，组件按新布局的比例重新初始化尺寸。
        let group_id = SharedString::from(format!("browser-split-{ids:?}-{structure:?}"));
        let mut area = self.resizable_group(h_resizable(group_id), SplitGroup::Columns, cx);
        let mut start = 0;
        for (index, column) in columns.into_iter().enumerate() {
            let width = widths[index];
            let minimum = MIN_ROW_HEIGHT.min(height / column.rows as f32);
            let top = (height * column.top_ratio).clamp(minimum, (height - minimum).max(minimum));
            let mut rows =
                self.resizable_group(v_resizable("browser-column"), SplitGroup::Rows(index), cx);
            for row in 0..column.rows {
                let height = match (column.rows, row) {
                    (1, _) => height,
                    (_, 0) => top,
                    _ => height - top,
                };
                let count = column.row_panes[row];
                let row_ids = &ids[start..start + count];
                // 内层最小宽度总和须能放进外层列的最小宽度。
                let row_minimum = px(100.).min(column_minimum / count as f32);
                let widths = row_extents(width, row_minimum, count, column.row_ratios[row]);
                let mut line = self.resizable_group(
                    h_resizable(("browser-row", row)),
                    SplitGroup::RowPanes(index, row),
                    cx,
                );
                for (position, &id) in row_ids.iter().enumerate() {
                    let pane = self
                        .with_pane(id, |app| {
                            app.browser_pane(
                                size(widths[position], height),
                                true,
                                id == active,
                                window,
                                cx,
                            )
                        })
                        .expect("窗格存在");
                    line = line.child(
                        resizable_panel()
                            .size(widths[position])
                            .size_range(row_minimum..Pixels::MAX)
                            .child(pane),
                    );
                }
                start += count;
                rows = rows.child(
                    resizable_panel()
                        .size(height)
                        .size_range(minimum..Pixels::MAX)
                        .child(line),
                );
            }
            area = area.child(
                resizable_panel()
                    .size(width)
                    .size_range(column_minimum..Pixels::MAX)
                    .child(rows),
            );
        }
        // 分屏组填满侧栏之后剩余的空间，避免按整窗宽度重新分配窗格。
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(area)
            .into_any_element()
    }

    fn resizable_group(
        &self,
        group: ResizablePanelGroup,
        target: SplitGroup,
        cx: &Context<Self>,
    ) -> ResizablePanelGroup {
        let owner = cx.weak_entity();
        group.on_resize(move |state, _, cx| {
            let sizes = state.read(cx).sizes().clone();
            let total: Pixels = sizes.iter().copied().sum();
            if total <= px(0.) {
                return;
            }
            let _ = owner.update(cx, |app, cx| {
                match target {
                    SplitGroup::Columns => {
                        for (column, size) in app.pane_layout.columns.iter_mut().zip(&sizes) {
                            column.width_ratio = *size / total;
                        }
                    }
                    SplitGroup::Rows(column) => {
                        app.pane_layout.columns[column].top_ratio = sizes[0] / total;
                    }
                    SplitGroup::RowPanes(column, row) => {
                        let ratios = &mut app.pane_layout.columns[column].row_ratios[row];
                        for (ratio, size) in ratios.iter_mut().zip(&sizes) {
                            *ratio = *size / total;
                        }
                    }
                }
                app.save_workspace(cx);
                cx.notify();
            });
        })
    }

    fn browser_pane(
        &mut self,
        size: Size<Pixels>,
        split: bool,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let pane = &self.panes[self.pane_target];
        let id = pane.id;
        let measured_size = pane.measured_size;
        let size = measured_size.filter(|_| split).unwrap_or(size);
        let owner = cx.weak_entity();
        let theme = cx.theme();
        let top_border = if active { theme.primary } else { theme.border };
        v_flex()
            .id(SharedString::from(id.to_string()))
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(split, |pane| {
                pane.size_full()
                    .on_prepaint(move |bounds, window, cx| {
                        if measured_size == Some(bounds.size) {
                            return;
                        }
                        // 网格列数和标签宽度使用组件实际分配的尺寸。
                        let owner = owner.clone();
                        window.defer(cx, move |_, cx| {
                            let _ = owner.update(cx, |app, cx| {
                                if let Some(index) = app.pane_index(id) {
                                    app.panes[index].measured_size = Some(bounds.size);
                                    cx.notify();
                                }
                            });
                        });
                    })
                    .child(div().h(px(2.)).flex_shrink_0().bg(top_border))
                    .capture_any_mouse_down(cx.listener(move |app, _, window, cx| {
                        if let Some(index) = app.pane_index(id)
                            && index != app.active_pane
                        {
                            if app.pane_changes_allowed() {
                                app.focus_pane(index, window, cx);
                            } else {
                                cx.stop_propagation();
                            }
                        }
                    }))
            })
            .when(!split, |pane| pane.flex_1())
            .on_drag_move::<TerminalResize>(cx.listener(move |app, event, window, cx| {
                if app.panes[app.pane_target].id == id {
                    app.resize_embedded_terminal(event, window, cx);
                }
            }))
            .when(split || self.show_tab_bar(), |pane| {
                pane.child(self.tab_bar(size.width, cx))
            })
            .child(self.file_area(size.width, window, cx))
            .children(self.embedded_terminal_panel(size.height))
    }
}

fn row_extents(width: Pixels, minimum: Pixels, count: usize, ratios: [f32; 3]) -> [Pixels; 3] {
    let sum: f32 = ratios[..count].iter().sum();
    let ratios = if sum.is_finite() && sum > 0. && ratios.iter().all(|ratio| *ratio >= 0.) {
        ratios.map(|ratio| ratio / sum)
    } else {
        [1. / count as f32; 3]
    };
    pane_extents(width, minimum, count, ratios)
}

fn column_extents(width: Pixels, columns: &[PaneColumn]) -> [Pixels; MAX_COLUMNS] {
    let mut ratios = [0.; MAX_COLUMNS];
    for (ratio, column) in ratios.iter_mut().zip(columns) {
        *ratio = column.width_ratio;
    }
    pane_extents(width, MIN_COLUMN_WIDTH, columns.len(), ratios)
}

fn pane_extents(extent: Pixels, minimum: Pixels, count: usize, ratios: [f32; 3]) -> [Pixels; 3] {
    let available = extent.max(px(0.));
    let minimum = minimum.min(available / count as f32);
    let mut sizes = [px(0.); 3];
    let mut remaining = available;
    for (index, size) in sizes[..count - 1].iter_mut().enumerate() {
        let upper = remaining - minimum * (count - index - 1) as f32;
        *size = (available * ratios[index]).clamp(minimum, upper.max(minimum));
        remaining -= *size;
    }
    sizes[count - 1] = remaining.max(px(0.));
    sizes
}

#[cfg(test)]
mod tests {
    use super::pane_extents;
    use gpui_kit::px;

    #[test]
    fn three_panes_share_space_equally_including_narrow_windows() {
        for extent in [px(430.), px(1200.)] {
            let sizes = pane_extents(extent, px(200.), 3, [1. / 3.; 3]);
            let expected = extent / 3.;
            assert!(
                sizes
                    .into_iter()
                    .all(|size| size > expected - px(0.01) && size < expected + px(0.01))
            );
        }
    }
}
