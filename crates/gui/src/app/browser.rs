use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

use super::App;
use gpui_kit::*;
use settings::config::tab::Location;
use vfs::{DirectoryWatch, Entry, EntryKind};

use daemon::browse::{ChildCount, ChildCountMonitor, DirectorySnapshot, FileSort, ReadSource};

const ESTIMATED_GRID_ROW_PADDING: Pixels = px(60.);

const RECENT_DIRECTORY_LIMIT: usize = 8;
const RECENT_ENTRY_LIMIT: usize = 20_000;

#[derive(Default)]
pub(super) struct RecentDirectories(VecDeque<RecentDirectory>);

struct RecentDirectory {
    path: PathBuf,
    snapshot: Arc<DirectorySnapshot>,
    visible: Vec<usize>,
    query: String,
    show_hidden: bool,
    sort: FileSort,
}

impl RecentDirectories {
    fn get(
        &mut self,
        path: &Path,
        browser: &Browser,
    ) -> Option<(Arc<DirectorySnapshot>, Vec<usize>)> {
        let index = self.0.iter().position(|entry| {
            entry.path == path
                && entry.query == browser.query
                && entry.show_hidden == browser.show_hidden
                && entry.sort == browser.sort
        })?;
        let entry = self.0.remove(index)?;
        let result = (entry.snapshot.clone(), entry.visible.clone());
        self.0.push_back(entry);
        Some(result)
    }

    fn insert(&mut self, entry: RecentDirectory) {
        self.0.retain(|cached| cached.path != entry.path);
        if entry.snapshot.directory.entries.len() <= RECENT_ENTRY_LIMIT {
            self.0.push_back(entry);
        }
        while self.0.len() > RECENT_DIRECTORY_LIMIT
            || self
                .0
                .iter()
                .map(|entry| entry.snapshot.directory.entries.len())
                .sum::<usize>()
                > RECENT_ENTRY_LIMIT
        {
            self.0.pop_front();
        }
    }

    pub(super) fn remove_under(&mut self, root: &Path) {
        self.0.retain(|entry| !entry.path.starts_with(root));
    }
}

pub(super) enum DirectoryState {
    Unavailable,
    Loading { show_message: bool },
    Ready(Arc<DirectorySnapshot>),
    Failed(String),
}

struct ReadTask {
    cancelled: Arc<AtomicBool>,
    _task: Task<()>,
}

pub(super) enum SelectedSize {
    Loading,
    Ready(u64),
    Unavailable,
}

struct SizeRead {
    path: PathBuf,
    revision: u64,
    size: SelectedSize,
    task: Option<ReadTask>,
}

impl Drop for ReadTask {
    fn drop(&mut self) {
        self.cancelled.store(true, Relaxed);
    }
}

#[derive(Default)]
pub(super) struct TabBrowseState {
    pub(super) search: String,
    selection: Selection,
    scroll: Option<ScrollAnchor>,
}

struct ScrollAnchor {
    path: PathBuf,
    position: usize,
}

#[derive(PartialEq)]
struct GridScrollLayout {
    columns: usize,
    tile_width: Pixels,
    icon_size: Pixels,
    revision: u64,
}

#[derive(Default)]
struct Selection {
    paths: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
}

#[derive(PartialEq, Eq)]
enum ChildCountScope {
    Visible(Range<usize>),
    Sorted,
}

pub(super) struct Browser {
    pub(super) state: DirectoryState,
    pub(super) query: String,
    pub(super) show_hidden: bool,
    pub(super) visible: Vec<usize>,
    sort: FileSort,
    child_counts: HashMap<usize, ChildCount>,
    child_count_scope: Option<ChildCountScope>,
    child_count_task: Option<ReadTask>,
    selection: Selection,
    pub(super) reveal_path: Option<PathBuf>,
    pub(super) reveal_row: Option<usize>,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) scroll_layout: Option<(usize, Pixels)>,
    pub(super) grid_scroll: ListState,
    grid_layout: Option<GridScrollLayout>,
    pub(super) restore_row: Option<usize>,
    pub(super) tab_restore: Option<TabBrowseState>,
    pub(super) action_error: Option<String>,
    pub(super) filtering: bool,
    pub(super) snapshot_revision: u64,
    read_task: Option<ReadTask>,
    directory_watch: Option<Task<()>>,
    refresh_pending: bool,
    filter_task: Option<ReadTask>,
    size_read: Option<SizeRead>,
}

impl Browser {
    fn snapshot(&self) -> Option<&DirectorySnapshot> {
        match &self.state {
            DirectoryState::Ready(snapshot) => Some(snapshot),
            _ => None,
        }
    }

    pub(super) fn entry(&self, index: usize) -> Option<&Entry> {
        self.snapshot()?.directory.entries.get(index)
    }

    pub(super) fn selection_len(&self) -> usize {
        self.selection.paths.len()
    }

    pub(super) fn selected_size(&self) -> Option<&SelectedSize> {
        self.size_read.as_ref().map(|read| &read.size)
    }

    pub(super) fn is_selected(&self, path: &Path) -> bool {
        self.selection.paths.contains(path)
    }

    pub(super) fn single_selected_entry(&self) -> Option<&Entry> {
        if self.selection_len() != 1 {
            return None;
        }
        let snapshot = self.snapshot()?;
        let path = self.selection.paths.first()?;
        snapshot.directory.entries.get(*snapshot.indices.get(path)?)
    }

    pub(super) fn selected_entries(&self) -> impl Iterator<Item = &Entry> {
        let snapshot = self.snapshot();
        let mut indices = self
            .selection
            .paths
            .iter()
            .filter_map(|path| snapshot?.indices.get(path).copied())
            .collect::<Vec<_>>();
        // 批量操作继续按列表顺序执行。
        indices.sort_unstable();
        indices
            .into_iter()
            .filter_map(move |index| self.entry(index))
    }

    pub(super) fn clear_selection(&mut self) {
        self.selection = Selection::default();
        self.reveal_path = None;
        self.reveal_row = None;
    }

    pub(super) fn select_all(&mut self) {
        self.clear_selection();
        let paths: BTreeSet<_> = self
            .visible
            .iter()
            .filter_map(|&index| self.entry(index).map(|entry| entry.path.clone()))
            .collect();
        self.selection.anchor = self
            .visible
            .first()
            .and_then(|&index| self.entry(index))
            .map(|entry| entry.path.clone());
        self.selection.paths = paths;
    }

    fn select_entry(&mut self, index: usize, modifiers: gpui_kit::Modifiers) {
        let Some(path) = self.entry(index).map(|entry| entry.path.clone()) else {
            return;
        };
        self.reveal_path = None;
        self.reveal_row = None;
        if modifiers.shift
            && let Some(start) = self
                .selection
                .anchor
                .as_deref()
                .and_then(|anchor| self.visible_position(anchor))
            && let Some(end) = self.visible.iter().position(|&entry| entry == index)
        {
            let paths: Vec<_> = self.visible[start.min(end)..=start.max(end)]
                .iter()
                .filter_map(|&index| self.entry(index).map(|entry| entry.path.clone()))
                .collect();
            if !modifiers.control {
                self.selection.paths.clear();
            }
            self.selection.paths.extend(paths);
            return;
        }
        if modifiers.control {
            if !self.selection.paths.remove(&path) {
                self.selection.paths.insert(path.clone());
            }
        } else {
            self.selection.paths.clear();
            self.selection.paths.insert(path.clone());
        }
        self.selection.anchor = Some(path);
    }

    pub(super) fn select_for_context_menu(&mut self, index: usize) -> bool {
        if self
            .entry(index)
            .is_none_or(|entry| self.is_selected(&entry.path))
        {
            return false;
        }
        self.select_entry(index, gpui_kit::Modifiers::default());
        true
    }

    fn prune_selection(&mut self, drop_missing: bool) {
        let DirectoryState::Ready(snapshot) = &self.state else {
            return;
        };
        if self.selection.paths.is_empty() && self.selection.anchor.is_none() {
            return;
        }
        let visible_indices: HashSet<_> = self.visible.iter().copied().collect();
        let visible = |path: &Path| {
            snapshot.indices.get(path).is_some_and(|index| {
                visible_indices.contains(index)
                    && (!drop_missing
                        || snapshot.directory.entries[*index].kind != EntryKind::Missing)
            })
        };
        self.selection.paths.retain(|path| visible(path));
        if self
            .selection
            .anchor
            .as_deref()
            .is_some_and(|path| !visible(path))
        {
            self.selection.anchor = None;
        }
    }

    pub(super) fn visible_entry_index(&self, path: &Path) -> Option<usize> {
        Some(self.visible[self.visible_position(path)?])
    }

    fn visible_position(&self, path: &Path) -> Option<usize> {
        let index = *self.snapshot()?.indices.get(path)?;
        self.visible.iter().position(|&entry| entry == index)
    }

    pub(super) fn child_count(&self, index: usize) -> Option<u64> {
        self.child_counts.get(&index)?.count
    }

    pub(super) fn stop_child_counts(&mut self) {
        self.child_count_scope = None;
        self.child_count_task = None;
    }

    fn replace_visible_order(&mut self, visible: Vec<usize>) {
        if self.visible != visible {
            let anchor = self.scroll_anchor();
            self.visible = visible;
            if let Some(anchor) = anchor {
                self.restore_scroll(&anchor);
            }
        }
    }

    fn scroll_anchor(&self) -> Option<ScrollAnchor> {
        let position = self.restore_row.unwrap_or_else(|| {
            if let Some(layout) = &self.grid_layout {
                return self.grid_scroll.logical_scroll_top().item_ix * layout.columns;
            }
            self.scroll_layout.map_or(0, |(columns, row_height)| {
                let offset = self.scroll.0.borrow().base_handle.offset();
                ((-offset.y / row_height).max(0.) as usize) * columns
            })
        });
        let index = *self.visible.get(position)?;
        Some(ScrollAnchor {
            path: self.entry(index)?.path.clone(),
            position,
        })
    }

    fn restore_scroll(&mut self, anchor: &ScrollAnchor) {
        self.restore_row = self.visible.len().checked_sub(1).map(|last| {
            self.visible_position(&anchor.path)
                .unwrap_or(anchor.position.min(last))
        });
    }

    pub(super) fn preserve_scroll_anchor(&mut self) {
        if let Some(anchor) = self.scroll_anchor() {
            self.restore_scroll(&anchor);
        }
    }

    pub(super) fn prepare_grid_scroll(
        &mut self,
        columns: usize,
        tile_width: Pixels,
        icon_size: Pixels,
    ) {
        let layout = GridScrollLayout {
            columns,
            tile_width,
            icon_size,
            revision: self.snapshot_revision,
        };
        let rows = self.visible.len().div_ceil(columns);
        if self.grid_layout.as_ref() != Some(&layout) || self.grid_scroll.item_count() != rows {
            if let Some(previous) = &self.grid_layout
                && previous.columns == columns
                && previous.revision == layout.revision
                && self.grid_scroll.item_count() == rows
            {
                self.grid_scroll.remeasure();
            } else {
                let previous_top = self.grid_layout.as_ref().map(|previous| {
                    let top = self.grid_scroll.logical_scroll_top();
                    ListOffset {
                        item_ix: top.item_ix * previous.columns / columns,
                        offset_in_item: top.offset_in_item,
                    }
                });
                // 列数改变后沿用原首行的文件定位，并保留行内滚动偏移。
                self.grid_scroll
                    .reset_with_uniform_height(rows, icon_size + ESTIMATED_GRID_ROW_PADDING);
                if let Some(top) = previous_top {
                    self.grid_scroll.scroll_to(top);
                }
            }
            self.grid_layout = Some(layout);
        }
    }

    pub(super) fn reset_grid_scroll(&mut self) {
        self.grid_scroll.reset(0);
        self.grid_layout = None;
    }

    pub(super) fn new(show_hidden: bool) -> Self {
        Self {
            state: DirectoryState::Unavailable,
            query: String::new(),
            show_hidden,
            visible: Vec::new(),
            sort: FileSort::default(),
            child_counts: HashMap::new(),
            child_count_scope: None,
            child_count_task: None,
            selection: Selection::default(),
            reveal_path: None,
            reveal_row: None,
            scroll: UniformListScrollHandle::new(),
            scroll_layout: None,
            grid_scroll: ListState::new(0, ListAlignment::Top, px(400.)),
            grid_layout: None,
            restore_row: None,
            tab_restore: None,
            action_error: None,
            filtering: false,
            snapshot_revision: 0,
            read_task: None,
            directory_watch: None,
            refresh_pending: false,
            filter_task: None,
            size_read: None,
        }
    }
}

impl App {
    pub(super) fn sort_directory(&mut self, sort: FileSort, cx: &mut Context<Self>) {
        self.panes[self.pane_target].browser.sort = sort;
        self.filter_directory(false, true, cx);
    }

    pub(super) fn update_child_count_scope(
        &mut self,
        rows: Range<usize>,
        size_visible: bool,
        cx: &mut Context<Self>,
    ) {
        let pane = &mut self.panes[self.pane_target];
        let pane_id = pane.id;
        let browser = &mut pane.browser;
        let scope = if browser.filtering || browser.visible.is_empty() {
            None
        } else if browser.sort.by_size() {
            Some(ChildCountScope::Sorted)
        } else if size_visible && rows.start < rows.end {
            Some(ChildCountScope::Visible(
                rows.start..rows.end.min(browser.visible.len()),
            ))
        } else {
            None
        };
        if browser.child_count_scope == scope {
            return;
        }
        browser.stop_child_counts();
        let (Some(scope), DirectoryState::Ready(snapshot)) = (scope, &browser.state) else {
            return;
        };
        let rows = match &scope {
            ChildCountScope::Visible(rows) => rows.clone(),
            ChildCountScope::Sorted => 0..browser.visible.len(),
        };
        let targets: Vec<_> = browser
            .visible
            .get(rows)
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|&index| snapshot.directory.entries[index].kind == EntryKind::Directory)
            .collect();
        let sorted = matches!(scope, ChildCountScope::Sorted);
        browser.child_count_scope = Some(scope);
        if targets.is_empty() {
            return;
        }

        let snapshot = snapshot.clone();
        let revision = browser.snapshot_revision;
        let sort = browser.sort;
        let visible = sorted.then(|| browser.visible.clone());
        let counts = targets
            .iter()
            .filter_map(|index| {
                browser
                    .child_counts
                    .get(index)
                    .map(|count| (*index, *count))
            })
            .collect();
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_cancelled = cancelled.clone();
        let executor = cx.background_executor().clone();
        let task = cx.spawn(async move |this, cx| {
            let setup_cancelled = task_cancelled.clone();
            let mut monitor = executor
                .spawn(async move {
                    ChildCountMonitor::new(
                        snapshot,
                        targets,
                        counts,
                        visible,
                        sort,
                        &setup_cancelled,
                    )
                })
                .await;
            while !task_cancelled.load(Relaxed) {
                let worker_cancelled = task_cancelled.clone();
                let (next_monitor, update) = executor
                    .spawn(async move {
                        let update = monitor.refresh(&worker_cancelled);
                        (monitor, update)
                    })
                    .await;
                monitor = next_monitor;
                let daemon::browse::ChildCountUpdate { changed, reordered } = update;
                if task_cancelled.load(Relaxed) {
                    break;
                }
                if !changed.is_empty() {
                    let applied = this
                        .update(cx, |this, cx| {
                            this.with_pane(pane_id, |this| {
                                let browser = &mut this.panes[this.pane_target].browser;
                                // 滚动、筛选或切换目录后，旧任务不能覆盖新范围。
                                if task_cancelled.load(Relaxed)
                                    || browser.snapshot_revision != revision
                                {
                                    return false;
                                }
                                browser.child_counts.extend(changed);
                                if let Some(visible) = reordered {
                                    browser.replace_visible_order(visible);
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(false)
                        })
                        .unwrap_or(false);
                    if !applied {
                        break;
                    }
                }
                if !monitor.wait_for_change().await {
                    break;
                }
            }
        });
        browser.child_count_task = Some(ReadTask {
            cancelled,
            _task: task,
        });
    }

    pub(super) fn refresh_selected_size(&mut self, cx: &mut Context<Self>) {
        let pane = &mut self.panes[self.pane_target];
        let browser = &mut pane.browser;
        let Some(entry) = browser.single_selected_entry() else {
            browser.size_read = None;
            return;
        };
        let revision = browser.snapshot_revision;
        if browser
            .size_read
            .as_ref()
            .is_some_and(|read| read.revision == revision && read.path == entry.path)
        {
            return;
        }

        let path = entry.path.clone();
        let pane_id = pane.id;
        let cancelled = Arc::new(AtomicBool::new(false));
        let read_cancelled = cancelled.clone();
        let request = cancelled.clone();
        let work_path = path.clone();
        let work = cx.background_spawn(async move {
            vfs::entry_size(&work_path, || read_cancelled.load(Relaxed))
        });
        let task = cx.spawn(async move |app, cx| {
            let size = match work.await {
                Ok(bytes) => SelectedSize::Ready(bytes),
                Err(_) => SelectedSize::Unavailable,
            };
            let _ = app.update(cx, |app, cx| {
                if let Some(pane) = app.panes.iter_mut().find(|pane| pane.id == pane_id)
                    && let Some(read) = &mut pane.browser.size_read
                    && read
                        .task
                        .as_ref()
                        .is_some_and(|task| Arc::ptr_eq(&task.cancelled, &request))
                {
                    read.size = size;
                    read.task = None;
                    cx.notify();
                }
            });
        });
        browser.size_read = Some(SizeRead {
            path,
            revision,
            size: SelectedSize::Loading,
            task: Some(ReadTask {
                cancelled,
                _task: task,
            }),
        });
    }

    pub(super) fn load_directory(&mut self, _window: &Window, cx: &mut Context<Self>) {
        let source = self.read_source(cx);
        let tag_pending = matches!(source, Some(ReadSource::MissingTag))
            .then(|| self.tag_state.read(cx))
            .filter(|state| state.loaded_tags().is_none())
            .map(|state| match state.load_error() {
                Some(error) => DirectoryState::Failed(error.to_owned()),
                None => DirectoryState::Loading { show_message: true },
            });
        let browser = &mut self.panes[self.pane_target].browser;
        browser.snapshot_revision += 1;
        browser.stop_child_counts();
        browser.child_counts.clear();
        browser.read_task = None;
        browser.directory_watch = None;
        browser.refresh_pending = false;
        browser.filter_task = None;
        browser.filtering = false;
        browser.visible = Vec::new();
        browser.clear_selection();
        browser.action_error = None;
        browser.scroll = UniformListScrollHandle::new();
        browser.reset_grid_scroll();
        browser.restore_row = None;
        let Some(source) = source else {
            browser.state = DirectoryState::Unavailable;
            cx.notify();
            return;
        };

        if let Some(state) = tag_pending {
            browser.state = state;
            cx.notify();
            return;
        }
        browser.state = DirectoryState::Loading {
            show_message: false,
        };
        if let ReadSource::Directory(path) = &source {
            if let Some((snapshot, visible)) = self.recent_directories.get(path, browser) {
                self.replace_snapshot(snapshot, visible);
            }
            self.watch_directory(path, cx);
        }
        self.read_snapshot(source, cx);
    }

    fn watch_directory(&mut self, path: &Path, cx: &mut Context<Self>) {
        let watch = match DirectoryWatch::new(path) {
            Ok(watch) => watch,
            Err(error) => {
                tracing::warn!(%error, "无法监听目录");
                return;
            }
        };
        let pane_id = self.panes[self.pane_target].id;
        let executor = cx.background_executor().clone();
        self.panes[self.pane_target].browser.directory_watch =
            Some(cx.spawn(async move |this, cx| {
                while watch.changed().await {
                    // 合并复制、解压等短时间内连续产生的文件系统事件。
                    executor.timer(Duration::from_millis(120)).await;
                    watch.coalesce();
                    if this
                        .update(cx, |this, cx| {
                            this.with_pane(pane_id, |this| this.refresh_directory(cx))
                        })
                        .ok()
                        .flatten()
                        .is_none()
                    {
                        break;
                    }
                }
            }));
    }

    pub(super) fn refresh_after_file_operation(
        &mut self,
        reveal: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.panes[self.pane_target].browser.read_task = None;
        self.panes[self.pane_target].browser.refresh_pending = false;
        self.panes[self.pane_target].browser.reveal_path = reveal;
        self.refresh_directory(cx);
    }

    fn read_source(&self, cx: &Context<Self>) -> Option<ReadSource> {
        self.current_location().map(|location| match location {
            Location::Directory { path } => ReadSource::Directory(path.0.clone()),
            Location::Tag { id } => self
                .tags(cx)
                .get(*id)
                .map_or(ReadSource::MissingTag, |tag| {
                    ReadSource::Tag(tag.paths.iter().map(|path| path.0.clone()).collect())
                }),
        })
    }

    pub(super) fn refresh_directory(&mut self, cx: &mut Context<Self>) {
        if self.panes[self.pane_target].browser.read_task.is_some() {
            self.panes[self.pane_target].browser.refresh_pending = true;
            return;
        }
        let Some(source) = self.read_source(cx) else {
            return;
        };
        if let ReadSource::Directory(path) = &source
            && self.panes[self.pane_target]
                .browser
                .directory_watch
                .is_none()
        {
            self.watch_directory(path, cx);
        }
        self.read_snapshot(source, cx);
    }

    fn read_snapshot(&mut self, source: ReadSource, cx: &mut Context<Self>) {
        let pane_id = self.panes[self.pane_target].id;
        let directory_path = match &source {
            ReadSource::Directory(path) => Some(path.clone()),
            _ => None,
        };
        let pane = &mut self.panes[self.pane_target];
        let browser = &mut pane.browser;
        let snapshot_revision = browser.snapshot_revision;
        let show_loading_later = matches!(
            browser.state,
            DirectoryState::Loading {
                show_message: false
            }
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let read_cancelled = cancelled.clone();
        let query = browser.query.clone();
        let sort = browser.sort;
        let show_hidden = browser.show_hidden;
        let read = cx.background_spawn(async move {
            let directory = DirectorySnapshot::read(source, &read_cancelled)?;
            let visible = directory.visible_entries(
                &query,
                show_hidden,
                sort,
                &HashMap::new(),
                &read_cancelled,
            );
            Ok::<_, std::io::Error>((directory, visible, query))
        });
        let task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.with_pane(pane_id, |this| {
                    if this.panes[this.pane_target].browser.snapshot_revision != snapshot_revision {
                        return;
                    }
                    this.panes[this.pane_target].browser.read_task = None;
                    let mut refresh_pending =
                        std::mem::take(&mut this.panes[this.pane_target].browser.refresh_pending);
                    match result {
                        Ok((directory, visible, query)) => {
                            let filter_matches = query
                                == this.panes[this.pane_target].browser.query
                                && show_hidden == this.panes[this.pane_target].browser.show_hidden
                                && sort == this.panes[this.pane_target].browser.sort;
                            if filter_matches {
                                let snapshot = Arc::new(directory);
                                if let Some(path) = directory_path {
                                    this.recent_directories.insert(RecentDirectory {
                                        path,
                                        snapshot: snapshot.clone(),
                                        visible: visible.clone(),
                                        query,
                                        show_hidden,
                                        sort,
                                    });
                                }
                                this.replace_snapshot(snapshot, visible);
                            } else if matches!(
                                this.panes[this.pane_target].browser.state,
                                DirectoryState::Ready(_)
                            ) {
                                // 旧索引仍对应当前快照，等后台按最新筛选条件读完再替换。
                                refresh_pending = true;
                            } else {
                                this.panes[this.pane_target].browser.state =
                                    DirectoryState::Ready(Arc::new(directory));
                                this.filter_directory(false, false, cx);
                            }
                        }
                        Err(error) => {
                            if let Some(path) = directory_path {
                                this.recent_directories.remove_under(&path);
                            }
                            let browser = &mut this.panes[this.pane_target].browser;
                            browser.snapshot_revision += 1;
                            browser.stop_child_counts();
                            browser.child_counts.clear();
                            browser.filter_task = None;
                            browser.filtering = false;
                            browser.visible.clear();
                            browser.clear_selection();
                            browser.state = DirectoryState::Failed(error.to_string());
                        }
                    }
                    if refresh_pending {
                        this.refresh_directory(cx);
                    }
                    cx.notify();
                });
            });
        });
        browser.read_task = Some(ReadTask {
            cancelled,
            _task: task,
        });
        if show_loading_later {
            let timer = cx.background_executor().timer(Duration::from_millis(120));
            cx.spawn(async move |app, cx| {
                timer.await;
                let _ = app.update(cx, |app, cx| {
                    app.with_pane(pane_id, |app| {
                        let browser = &mut app.panes[app.pane_target].browser;
                        if browser.snapshot_revision == snapshot_revision
                            && let DirectoryState::Loading { show_message } = &mut browser.state
                        {
                            *show_message = true;
                            cx.notify();
                        }
                    });
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn replace_snapshot(&mut self, snapshot: Arc<DirectorySnapshot>, visible: Vec<usize>) {
        let browser = &mut self.panes[self.pane_target].browser;
        // 刷新时按路径保留滚动锚点，文件插入或移除后仍能回到附近。
        let scroll = browser.scroll_anchor();
        browser.snapshot_revision += 1;
        browser.filter_task = None;
        browser.filtering = false;
        browser.state = DirectoryState::Ready(snapshot);
        browser.stop_child_counts();
        browser.child_counts.clear();
        browser.visible = visible;
        browser.prune_selection(false);
        if let Some(anchor) = scroll
            && browser.visible_position(&anchor.path) != Some(anchor.position)
        {
            browser.restore_scroll(&anchor);
        }
        self.restore_snapshot_position();
    }

    pub(super) fn take_tab_browse_state(&mut self, search: String) -> TabBrowseState {
        let browser = &mut self.panes[self.pane_target].browser;
        // 快速切换时，新快照可能还没到，继续保留未恢复的状态。
        if let Some(mut pending) = browser.tab_restore.take() {
            pending.search = search;
            return pending;
        }
        let scroll = browser.scroll_anchor();
        TabBrowseState {
            search,
            selection: std::mem::take(&mut browser.selection),
            scroll,
        }
    }

    fn restore_snapshot_position(&mut self) {
        let browser = &mut self.panes[self.pane_target].browser;
        if let Some(saved) = browser.tab_restore.take() {
            browser.selection = saved.selection;
            browser.prune_selection(true);
            if let Some(anchor) = saved.scroll {
                browser.restore_scroll(&anchor);
            }
        }
        if let Some(path) = browser.reveal_path.take()
            && let Some(position) = browser.visible_position(&path)
        {
            browser.selection.paths = BTreeSet::from([path.clone()]);
            browser.selection.anchor = Some(path);
            browser.restore_row = None;
            browser.reveal_row = Some(position);
        }
    }

    pub(super) fn search_directory(&mut self, query: String, cx: &mut Context<Self>) {
        if self.panes[self.pane_target].browser.query != query {
            self.panes[self.pane_target].browser.query = query;
            self.filter_directory(true, false, cx);
        }
    }

    pub(super) fn set_show_hidden_files(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.panes[self.pane_target].browser.show_hidden != show {
            self.panes[self.pane_target].browser.show_hidden = show;
            self.filter_directory(false, false, cx);
            cx.notify();
        }
    }

    fn filter_directory(&mut self, debounce: bool, preserve_scroll: bool, cx: &mut Context<Self>) {
        let pane_id = self.panes[self.pane_target].id;
        let browser = &mut self.panes[self.pane_target].browser;
        let DirectoryState::Ready(directory) = &browser.state else {
            return;
        };
        let directory = directory.clone();
        let snapshot_revision = browser.snapshot_revision;
        let query = browser.query.clone();
        let show_hidden = browser.show_hidden;
        let sort = browser.sort;
        let counts = if sort.by_size() {
            browser.child_counts.clone()
        } else {
            HashMap::new()
        };
        browser.stop_child_counts();
        let cancelled = Arc::new(AtomicBool::new(false));
        let filter_cancelled = cancelled.clone();
        let obsolete = cancelled.clone();
        let executor = cx.background_executor().clone();
        browser.filter_task = None;
        browser.filtering = true;
        let task = cx.spawn(async move |this, cx| {
            if debounce {
                executor.timer(Duration::from_millis(150)).await;
            }
            let visible = executor
                .spawn(async move {
                    directory.visible_entries(&query, show_hidden, sort, &counts, &filter_cancelled)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.with_pane(pane_id, |this| {
                    let browser = &mut this.panes[this.pane_target].browser;
                    if obsolete.load(Relaxed) || browser.snapshot_revision != snapshot_revision {
                        return;
                    }
                    if preserve_scroll {
                        browser.replace_visible_order(visible);
                    } else {
                        browser.visible = visible;
                        browser.scroll = UniformListScrollHandle::new();
                        browser.reset_grid_scroll();
                        browser.restore_row = None;
                    }
                    browser.prune_selection(false);
                    browser.filtering = false;
                    this.restore_snapshot_position();
                    cx.notify();
                });
            });
        });
        browser.filter_task = Some(ReadTask {
            cancelled,
            _task: task,
        });
        cx.notify();
    }

    pub(super) fn entry_target(&self, index: usize) -> Option<(PathBuf, EntryKind)> {
        let entry = self.panes[self.pane_target].browser.entry(index)?;
        Some((entry.path.clone(), entry.kind))
    }

    pub(super) fn activate_entry(
        &mut self,
        index: usize,
        click_count: usize,
        modifiers: gpui_kit::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, kind)) = self.entry_target(index) else {
            return;
        };
        if modifiers.shift || modifiers.control {
            self.panes[self.pane_target]
                .browser
                .select_entry(index, modifiers);
            cx.notify();
            return;
        }
        let activation = cx
            .global::<super::preferences::ActivePreferences>()
            .0
            .general
            .activation_policy;
        let clicks = match activation {
            settings::config::general::ActivationPolicy::Single => 1,
            settings::config::general::ActivationPolicy::Double => 2,
        };
        if kind == EntryKind::Directory && click_count == clicks {
            self.navigate(path, window, cx);
            return;
        }
        self.panes[self.pane_target]
            .browser
            .select_entry(index, modifiers);
        cx.notify();
        if click_count != clicks || !matches!(kind, EntryKind::File | EntryKind::Symlink) {
            return;
        }

        self.open_file_target(path, kind, window, cx);
    }

    pub(super) fn open_file_target(
        &mut self,
        path: PathBuf,
        kind: EntryKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.panes[self.pane_target].id;
        let navigation_revision = self.panes[self.pane_target].navigation_revision;
        self.panes[self.pane_target].browser.action_error = None;
        cx.notify();
        let open = cx.background_spawn(async move {
            if kind == EntryKind::Symlink {
                let target = path.metadata()?;
                if target.is_dir() {
                    return Ok(Some(path));
                }
                if !target.is_file() {
                    return Err(std::io::Error::other("链接目标不是普通文件或目录"));
                }
            }
            sys::opener::open(&path)?;
            Ok::<_, std::io::Error>(None)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = open.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.with_pane(pane_id, |this| {
                    if this.panes[this.pane_target].navigation_revision != navigation_revision {
                        return;
                    }
                    match result {
                        Ok(Some(path)) => this.navigate(path, window, cx),
                        Ok(None) => {}
                        Err(error) => {
                            this.panes[this.pane_target].browser.action_error =
                                Some(format!("无法打开：{error}"));
                            cx.notify();
                        }
                    }
                });
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::Modifiers;
    use std::{path::Path, sync::Arc};

    use super::{Browser, DirectoryState, RecentDirectories, RecentDirectory};
    use vfs::{Directory, Entry, EntryKind};

    fn browser_with_entries() -> Browser {
        let mut browser = Browser::new(false);
        let entries = (0..9)
            .map(|index| Entry {
                name: index.to_string().into(),
                path: format!("/{index}").into(),
                kind: EntryKind::File,
                size: None,
                modified: None,
                created: None,
                accessed: None,
                access: None,
            })
            .collect();
        browser.state = super::DirectoryState::Ready(
            super::DirectorySnapshot::new(Directory {
                entries,
                read_errors: 0,
            })
            .into(),
        );
        browser.visible = vec![0, 2, 5, 8];
        browser
    }

    fn selected_names(browser: &Browser) -> Vec<&str> {
        browser
            .selected_entries()
            .map(|entry| entry.name.to_str().unwrap())
            .collect()
    }

    #[test]
    fn reuses_recent_directory_only_with_matching_view_and_fresh_snapshot() {
        let mut browser = browser_with_entries();
        let DirectoryState::Ready(snapshot) = &browser.state else {
            panic!("测试目录应已加载");
        };
        let snapshot = snapshot.clone();
        let path = Path::new("/mnt/pictures");
        let mut cache = RecentDirectories::default();
        cache.insert(RecentDirectory {
            path: path.to_owned(),
            snapshot: snapshot.clone(),
            visible: browser.visible.clone(),
            query: browser.query.clone(),
            show_hidden: browser.show_hidden,
            sort: browser.sort,
        });

        let (cached, visible) = cache.get(path, &browser).unwrap();
        assert!(Arc::ptr_eq(&cached, &snapshot));
        assert_eq!(visible, browser.visible);
        browser.show_hidden = true;
        assert!(cache.get(path, &browser).is_none());
        browser.show_hidden = false;
        assert!(cache.get(path, &browser).is_some());
    }

    #[test]
    fn range_selection_follows_reordered_rows() {
        let mut browser = browser_with_entries();
        browser.visible = vec![5, 0, 8, 2];
        browser.select_entry(5, Modifiers::default());
        browser.select_entry(
            8,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        );
        assert_eq!(selected_names(&browser), ["0", "5", "8"]);
        assert_eq!(
            browser.visible_position(std::path::Path::new("/0")),
            Some(1)
        );
    }

    #[test]
    fn grid_reflow_keeps_the_top_visible_entry_as_scroll_anchor() {
        use gpui_kit::{ListOffset, px};

        let mut browser = browser_with_entries();
        browser.prepare_grid_scroll(2, px(112.), px(64.));
        browser.grid_scroll.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(11.),
        });
        assert_eq!(browser.scroll_anchor().unwrap().position, 2);

        browser.prepare_grid_scroll(1, px(144.), px(96.));
        assert_eq!(browser.grid_scroll.item_count(), 4);
        let top = browser.grid_scroll.logical_scroll_top();
        assert_eq!(top.item_ix, 2);
        assert_eq!(top.offset_in_item, px(11.));
    }

    #[test]
    fn range_selection_respects_filter_and_control_click_toggles() {
        let mut browser = browser_with_entries();
        browser.select_entry(2, Modifiers::default());
        browser.select_entry(
            8,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(selected_names(&browser), ["2", "5", "8"]);
        browser.select_entry(
            5,
            Modifiers {
                control: true,
                ..Default::default()
            },
        );
        assert_eq!(selected_names(&browser), ["2", "8"]);
        browser.clear_selection();
        assert_eq!(browser.selection_len(), 0);
        assert_eq!(browser.selection.anchor, None);
    }

    #[test]
    fn context_menu_preserves_selected_group_and_replaces_it_for_other_entries() {
        let mut browser = browser_with_entries();
        browser.select_entry(2, Modifiers::default());
        browser.select_entry(
            5,
            Modifiers {
                control: true,
                ..Default::default()
            },
        );

        assert!(!browser.select_for_context_menu(5));
        assert_eq!(selected_names(&browser), ["2", "5"]);

        assert!(browser.select_for_context_menu(8));
        assert_eq!(selected_names(&browser), ["8"]);
        assert_eq!(
            browser.selection.anchor.as_deref(),
            Some(std::path::Path::new("/8"))
        );
    }

    #[test]
    fn selection_follows_paths_after_reordering_and_drops_hidden_entries() {
        let mut browser = browser_with_entries();
        browser.select_all();
        let entries = ["8", "2", "5", "0"]
            .into_iter()
            .map(|name| Entry {
                name: name.into(),
                path: format!("/{name}").into(),
                kind: EntryKind::File,
                size: None,
                modified: None,
                created: None,
                accessed: None,
                access: None,
            })
            .collect();
        browser.state = super::DirectoryState::Ready(
            super::DirectorySnapshot::new(Directory {
                entries,
                read_errors: 0,
            })
            .into(),
        );
        browser.visible = vec![0, 1, 3];
        browser.prune_selection(false);

        assert_eq!(selected_names(&browser), ["8", "2", "0"]);
        assert_eq!(
            browser.selection.anchor.as_deref(),
            Some(std::path::Path::new("/0"))
        );
        assert_eq!(
            browser.visible_entry_index(std::path::Path::new("/8")),
            Some(0)
        );
        assert_eq!(
            browser.visible_entry_index(std::path::Path::new("/5")),
            None
        );
        browser.restore_scroll(&super::ScrollAnchor {
            path: "/0".into(),
            position: 0,
        });
        assert_eq!(browser.restore_row, Some(2));
    }
}
