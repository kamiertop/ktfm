use std::cell::Cell;

use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::{
    tab::{GridSession, PaneColumn, SplitLayout},
    view::ViewMode,
};
use uuid::Uuid;

use super::App;
use super::browser::Browser;
use super::embed_terminal::TerminalResize;
use super::preferences::ActivePreferences;
use super::tabs::{BrowserTab, restore_tab_items};
use super::tags::TagChange;
use crate::style::{SIDEBAR_WIDTH, TITLE_BAR_HEIGHT};

const DIVIDER: Pixels = px(6.);
const MAX_COLUMNS: usize = 3;
const MAX_PANES: usize = 6;

mod layout;
pub(super) use layout::PaneLayout;
const MIN_COLUMN_WIDTH: Pixels = px(200.);
const MIN_ROW_HEIGHT: Pixels = px(140.);

pub(super) struct BrowserPane {
    pub(super) id: Uuid,
    pub(super) tabs: Vec<BrowserTab>,
    pub(super) active_tab: usize,
    pub(super) navigation_revision: u64,
    pub(super) search: Entity<InputState>,
    pub(super) browser: Browser,
    pub(super) view_mode: ViewMode,
}

impl BrowserPane {
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
            tabs,
            active_tab,
            navigation_revision: 0,
            search,
            browser: Browser::new(show_hidden),
            view_mode,
        }
    }
}

struct PaneResize {
    target: ResizeTarget,
    divider: usize,
    offset: Cell<Pixels>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResizeTarget {
    Columns,
    Rows(usize),
    RowPanes(usize, usize),
}

struct ResizePreview;

impl Render for ResizePreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
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

    fn pane_changes_allowed(&self) -> bool {
        self.file_name_editor.is_none()
            && self.directory_properties.is_none()
            && self.tag_editor.is_none()
            && self.bookmark_rename.is_none()
            && self.bookmark_properties.is_none()
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
            self.panes
                .push(BrowserPane::new(tabs, selected, window, cx));
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
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewport = window.viewport_size();
        let width = (viewport.width - SIDEBAR_WIDTH - border_width * 2.).max(px(0.));
        let height = (viewport.height - TITLE_BAR_HEIGHT - border_width * 2.).max(px(0.));
        if self.panes.len() == 1 {
            return self
                .browser_pane(size(width, height), false, true, cx)
                .into_any_element();
        }

        let active = self.panes[self.pane_target].id;
        let ids: Vec<_> = self.ordered_panes().map(|pane| pane.id).collect();
        let columns = self.pane_layout.columns.clone();
        let widths = column_extents(width, &columns);
        let mut start = 0;
        let mut area = h_flex()
            .id("browser-split")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .items_stretch()
            .on_drag_move::<PaneResize>(cx.listener(
                |app, event: &DragMoveEvent<PaneResize>, _, cx| {
                    let drag = event.drag(cx);
                    let count = app.pane_layout.columns.len();
                    if drag.target != ResizeTarget::Columns || drag.divider + 1 >= count {
                        return;
                    }
                    let divider = drag.divider;
                    let extent = event.bounds.size.width;
                    let available = extent - DIVIDER * (count - 1) as f32;
                    if available <= MIN_COLUMN_WIDTH * count as f32 {
                        return;
                    }
                    let mut sizes = column_extents(extent, &app.pane_layout.columns);
                    let preceding: Pixels = sizes[..divider].iter().copied().sum();
                    let pair = sizes[divider] + sizes[divider + 1];
                    let position = event.event.position.x
                        - event.bounds.left()
                        - drag.offset.get()
                        - DIVIDER * divider as f32
                        - preceding;
                    sizes[divider] = position.clamp(
                        MIN_COLUMN_WIDTH,
                        (pair - MIN_COLUMN_WIDTH).max(MIN_COLUMN_WIDTH),
                    );
                    sizes[divider + 1] = pair - sizes[divider];
                    for (column, size) in app.pane_layout.columns.iter_mut().zip(sizes) {
                        column.width_ratio = size / available;
                    }
                    cx.notify();
                },
            ))
            .on_drop(cx.listener(|app, _: &PaneResize, _, cx| app.save_workspace(cx)));
        for (index, column) in columns.into_iter().enumerate() {
            if index != 0 {
                area = area.child(pane_divider(ResizeTarget::Columns, index - 1, cx));
            }
            let width = widths[index];
            let mut element = v_flex()
                .id(("browser-column", index))
                .w(width)
                .h(height)
                .items_stretch()
                .flex_shrink_0()
                .min_w_0()
                .min_h_0()
                .on_drag_move::<PaneResize>(cx.listener(
                    move |app, event: &DragMoveEvent<PaneResize>, _, cx| {
                        let drag = event.drag(cx);
                        let available = event.bounds.size.height - DIVIDER;
                        if drag.target != ResizeTarget::Rows(index)
                            || available <= MIN_ROW_HEIGHT * 2.
                        {
                            return;
                        }
                        let position =
                            event.event.position.y - event.bounds.top() - drag.offset.get();
                        if let Some(column) = app
                            .pane_layout
                            .columns
                            .get_mut(index)
                            .filter(|column| column.rows == 2)
                        {
                            column.top_ratio = position
                                .clamp(MIN_ROW_HEIGHT, available - MIN_ROW_HEIGHT)
                                / available;
                            cx.notify();
                        }
                    },
                ));
            let available = (height - DIVIDER).max(px(0.));
            let minimum = MIN_ROW_HEIGHT.min(available / 2.);
            let top = (available * column.top_ratio).clamp(minimum, available - minimum);
            for row in 0..column.rows {
                if row != 0 {
                    element = element.child(pane_divider(ResizeTarget::Rows(index), 0, cx));
                }
                let height = match (column.rows, row) {
                    (1, _) => height,
                    (_, 0) => top,
                    _ => available - top,
                };
                let count = column.row_panes[row];
                let target = ResizeTarget::RowPanes(index, row);
                let widths = row_extents(width, count, column.row_ratios[row]);
                let mut line = h_flex()
                    .id(("browser-row", row))
                    .w_full()
                    .h(height)
                    .flex_shrink_0()
                    .items_stretch()
                    .on_drag_move::<PaneResize>(cx.listener(
                        move |app, event: &DragMoveEvent<PaneResize>, _, cx| {
                            let drag = event.drag(cx);
                            let divider = drag.divider;
                            let extent = event.bounds.size.width;
                            let available = extent - DIVIDER * (count - 1) as f32;
                            let minimum = px(100.);
                            if drag.target != target
                                || divider + 1 >= count
                                || available <= minimum * count as f32
                            {
                                return;
                            }
                            let ratios = &mut app.pane_layout.columns[index].row_ratios[row];
                            let mut widths = row_extents(extent, count, *ratios);
                            let preceding: Pixels = widths[..divider].iter().copied().sum();
                            let pair = widths[divider] + widths[divider + 1];
                            let position = event.event.position.x
                                - event.bounds.left()
                                - drag.offset.get()
                                - DIVIDER * divider as f32
                                - preceding;
                            widths[divider] =
                                position.clamp(minimum, (pair - minimum).max(minimum));
                            widths[divider + 1] = pair - widths[divider];
                            *ratios = widths.map(|width| width / available);
                            cx.notify();
                        },
                    ));
                for (position, &id) in ids[start..start + count].iter().enumerate() {
                    if position != 0 {
                        line = line.child(pane_divider(target, position - 1, cx));
                    }
                    let pane = self
                        .with_pane(id, |app| {
                            app.browser_pane(size(widths[position], height), true, id == active, cx)
                        })
                        .expect("窗格存在");
                    line = line.child(pane);
                }
                start += count;
                element = element.child(line);
            }
            area = area.child(element);
        }
        area.into_any_element()
    }

    fn browser_pane(
        &mut self,
        size: Size<Pixels>,
        split: bool,
        active: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = self.panes[self.pane_target].id;
        let theme = cx.theme();
        let top_border = if active { theme.primary } else { theme.border };
        v_flex()
            .id(SharedString::from(id.to_string()))
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(split, |pane| {
                pane.w(size.width)
                    .h(size.height)
                    .flex_shrink_0()
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
            .child(self.file_area(size.width, cx))
            .children(self.embedded_terminal_panel(size.height))
    }
}

fn pane_divider(
    target: ResizeTarget,
    divider: usize,
    cx: &Context<App>,
) -> impl IntoElement + use<> {
    let vertical = matches!(target, ResizeTarget::Rows(_));
    let border = cx.theme().border;
    let accent = cx.theme().accent;
    div()
        .id(("pane-divider", divider))
        .when(vertical, |divider| {
            divider.h(DIVIDER).cursor(CursorStyle::ResizeUpDown)
        })
        .when(!vertical, |divider| {
            divider.w(DIVIDER).cursor(CursorStyle::ResizeLeftRight)
        })
        .flex_shrink_0()
        .bg(border)
        .hover(move |style| style.bg(accent))
        .on_drag(
            PaneResize {
                target,
                divider,
                offset: Cell::new(px(0.)),
            },
            move |drag, offset, _, cx| {
                drag.offset.set(if vertical { offset.y } else { offset.x });
                cx.new(|_| ResizePreview)
            },
        )
}

fn row_extents(width: Pixels, count: usize, ratios: [f32; 3]) -> [Pixels; 3] {
    let sum: f32 = ratios[..count].iter().sum();
    let ratios = if sum.is_finite() && sum > 0. && ratios.iter().all(|ratio| *ratio >= 0.) {
        ratios.map(|ratio| ratio / sum)
    } else {
        [1. / count as f32; 3]
    };
    pane_extents(width, px(100.), count, ratios)
}

fn column_extents(width: Pixels, columns: &[PaneColumn]) -> [Pixels; MAX_COLUMNS] {
    let mut ratios = [0.; MAX_COLUMNS];
    for (ratio, column) in ratios.iter_mut().zip(columns) {
        *ratio = column.width_ratio;
    }
    pane_extents(width, MIN_COLUMN_WIDTH, columns.len(), ratios)
}

fn pane_extents(extent: Pixels, minimum: Pixels, count: usize, ratios: [f32; 3]) -> [Pixels; 3] {
    let available = (extent - DIVIDER * (count - 1) as f32).max(px(0.));
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
    use super::{DIVIDER, pane_extents};
    use gpui_kit::px;

    #[test]
    fn three_panes_share_space_equally_including_narrow_windows() {
        for extent in [px(430.), px(1200.)] {
            let sizes = pane_extents(extent, px(200.), 3, [1. / 3.; 3]);
            let expected = (extent - DIVIDER * 2.) / 3.;
            assert!(
                sizes
                    .into_iter()
                    .all(|size| size > expected - px(0.01) && size < expected + px(0.01))
            );
        }
    }
}
