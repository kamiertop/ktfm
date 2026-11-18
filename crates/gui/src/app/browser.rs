use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

use gpui_kit::*;
use settings::config::tab::Location;
use vfs::{Directory, DirectoryWatch, Entry, EntryKind, read_directory, read_references};

use super::App;

pub(super) enum DirectoryState {
    Unavailable,
    Loading,
    Ready(Arc<DirectorySnapshot>),
    Failed(String),
}

pub(super) struct DirectorySnapshot {
    pub(super) directory: Directory,
    indices: HashMap<PathBuf, usize>,
}

impl DirectorySnapshot {
    fn new(directory: Directory) -> Self {
        let indices = directory
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect();
        Self { directory, indices }
    }
}

enum ReadSource {
    Directory(PathBuf),
    Tag(Vec<PathBuf>),
    MissingTag,
}

struct ReadTask {
    cancelled: Arc<AtomicBool>,
    _task: Task<()>,
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

#[derive(Default)]
struct Selection {
    paths: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
}

pub(super) struct Browser {
    pub(super) state: DirectoryState,
    pub(super) query: String,
    pub(super) show_hidden: bool,
    pub(super) visible: Vec<usize>,
    selection: Selection,
    pub(super) reveal_path: Option<PathBuf>,
    pub(super) reveal_row: Option<usize>,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) scroll_layout: Option<(usize, Pixels)>,
    pub(super) restore_row: Option<usize>,
    pub(super) tab_restore: Option<TabBrowseState>,
    pub(super) action_error: Option<String>,
    pub(super) filtering: bool,
    pub(super) snapshot_revision: u64,
    read_task: Option<ReadTask>,
    directory_watch: Option<Task<()>>,
    refresh_pending: bool,
    filter_task: Option<ReadTask>,
}

impl Browser {
    fn snapshot(&self) -> Option<&DirectorySnapshot> {
        match &self.state {
            DirectoryState::Ready(snapshot) => Some(snapshot),
            _ => None,
        }
    }

    fn entry(&self, index: usize) -> Option<&Entry> {
        self.snapshot()?.directory.entries.get(index)
    }

    pub(super) fn selection_len(&self) -> usize {
        self.selection.paths.len()
    }

    pub(super) fn is_selected(&self, path: &Path) -> bool {
        self.selection.paths.contains(path)
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
                .as_ref()
                .and_then(|anchor| self.snapshot()?.indices.get(anchor))
                .and_then(|anchor| self.visible.binary_search(anchor).ok())
            && let Ok(end) = self.visible.binary_search(&index)
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
        let visible = |path: &Path| {
            snapshot.indices.get(path).is_some_and(|index| {
                self.visible.binary_search(index).is_ok()
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
        self.visible
            .binary_search(self.snapshot()?.indices.get(path)?)
            .ok()
    }

    fn scroll_anchor(&self) -> Option<ScrollAnchor> {
        let position = self.restore_row.unwrap_or_else(|| {
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

    pub(super) fn new(show_hidden: bool) -> Self {
        Self {
            state: DirectoryState::Unavailable,
            query: String::new(),
            show_hidden,
            visible: Vec::new(),
            selection: Selection::default(),
            reveal_path: None,
            reveal_row: None,
            scroll: UniformListScrollHandle::new(),
            scroll_layout: None,
            restore_row: None,
            tab_restore: None,
            action_error: None,
            filtering: false,
            snapshot_revision: 0,
            read_task: None,
            directory_watch: None,
            refresh_pending: false,
            filter_task: None,
        }
    }
}

impl App {
    pub(super) fn load_directory(&mut self, cx: &mut Context<Self>) {
        let source = self.read_source(cx);
        let tag_pending = matches!(source, Some(ReadSource::MissingTag))
            .then(|| self.tag_state.read(cx))
            .filter(|state| state.loaded_tags().is_none())
            .map(|state| match state.load_error() {
                Some(error) => DirectoryState::Failed(error.to_owned()),
                None => DirectoryState::Loading,
            });
        let browser = &mut self.panes[self.pane_target].browser;
        browser.snapshot_revision += 1;
        browser.read_task = None;
        browser.directory_watch = None;
        browser.refresh_pending = false;
        browser.filter_task = None;
        browser.filtering = false;
        browser.visible = Vec::new();
        browser.clear_selection();
        browser.action_error = None;
        browser.scroll = UniformListScrollHandle::new();
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
        browser.state = DirectoryState::Loading;
        if let ReadSource::Directory(path) = &source {
            self.watch_directory(path, cx);
        }
        self.read_snapshot(source, cx);
    }

    fn watch_directory(&mut self, path: &Path, cx: &mut Context<Self>) {
        let watch = match DirectoryWatch::new(path) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("无法监听目录 {}：{error}", path.display());
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
        let browser = &mut self.panes[self.pane_target].browser;
        let snapshot_revision = browser.snapshot_revision;
        let cancelled = Arc::new(AtomicBool::new(false));
        let read_cancelled = cancelled.clone();
        let query = browser.query.clone();
        let show_hidden = browser.show_hidden;
        let read = cx.background_spawn(async move {
            let is_cancelled = || read_cancelled.load(Relaxed);
            let directory = match source {
                ReadSource::Directory(path) => read_directory(&path, is_cancelled)?,
                ReadSource::Tag(paths) => read_references(paths, is_cancelled)?,
                ReadSource::MissingTag => return Err(std::io::Error::other("标签已删除")),
            };
            let visible = filtered_entries(&directory, &query, show_hidden, &read_cancelled);
            Ok::<_, std::io::Error>((DirectorySnapshot::new(directory), visible, query))
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
                                && show_hidden == this.panes[this.pane_target].browser.show_hidden;
                            if filter_matches {
                                this.replace_snapshot(directory, visible);
                            } else if matches!(
                                this.panes[this.pane_target].browser.state,
                                DirectoryState::Ready(_)
                            ) {
                                // 旧索引仍对应当前快照，等后台按最新筛选条件读完再替换。
                                refresh_pending = true;
                            } else {
                                this.panes[this.pane_target].browser.state =
                                    DirectoryState::Ready(Arc::new(directory));
                                this.filter_directory(false, cx);
                            }
                        }
                        Err(error) => {
                            let browser = &mut this.panes[this.pane_target].browser;
                            browser.snapshot_revision += 1;
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
        cx.notify();
    }

    fn replace_snapshot(&mut self, snapshot: DirectorySnapshot, visible: Vec<usize>) {
        let browser = &mut self.panes[self.pane_target].browser;
        // 刷新时按路径保留滚动锚点，文件插入或移除后仍能回到附近。
        let scroll = browser.scroll_anchor();
        browser.snapshot_revision += 1;
        browser.filter_task = None;
        browser.filtering = false;
        browser.state = DirectoryState::Ready(Arc::new(snapshot));
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
            self.filter_directory(true, cx);
        }
    }

    pub(super) fn set_show_hidden_files(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.panes[self.pane_target].browser.show_hidden != show {
            self.panes[self.pane_target].browser.show_hidden = show;
            self.filter_directory(false, cx);
            cx.notify();
        }
    }

    fn filter_directory(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let pane_id = self.panes[self.pane_target].id;
        let browser = &mut self.panes[self.pane_target].browser;
        let DirectoryState::Ready(directory) = &browser.state else {
            return;
        };
        let directory = directory.clone();
        let snapshot_revision = browser.snapshot_revision;
        let query = browser.query.clone();
        let show_hidden = browser.show_hidden;
        let cancelled = Arc::new(AtomicBool::new(false));
        let filter_cancelled = cancelled.clone();
        let executor = cx.background_executor().clone();
        browser.filter_task = None;
        browser.filtering = true;
        let task = cx.spawn(async move |this, cx| {
            if debounce {
                executor.timer(Duration::from_millis(150)).await;
            }
            let visible = executor
                .spawn(async move {
                    filtered_entries(&directory.directory, &query, show_hidden, &filter_cancelled)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.with_pane(pane_id, |this| {
                    if this.panes[this.pane_target].browser.snapshot_revision == snapshot_revision {
                        this.panes[this.pane_target].browser.visible = visible;
                        this.panes[this.pane_target].browser.prune_selection(false);
                        this.panes[this.pane_target].browser.filtering = false;
                        this.panes[this.pane_target].browser.scroll =
                            UniformListScrollHandle::new();
                        this.panes[this.pane_target].browser.restore_row = None;
                        this.restore_snapshot_position();
                        cx.notify();
                    }
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

fn filtered_entries(
    directory: &Directory,
    query: &str,
    show_hidden: bool,
    cancelled: &AtomicBool,
) -> Vec<usize> {
    directory
        .entries
        .iter()
        .enumerate()
        .take_while(|_| !cancelled.load(Relaxed))
        .filter(|(_, entry)| {
            (show_hidden || !entry.is_hidden())
                && (query.is_empty() || entry.name.to_string_lossy().to_lowercase().contains(query))
        })
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use gpui_kit::Modifiers;

    use super::{AtomicBool, Browser, Relaxed, filtered_entries};
    use vfs::{Directory, Entry, EntryKind};

    fn browser_with_entries() -> Browser {
        let mut browser = Browser::new(false);
        let entries = (0..9)
            .map(|index| Entry {
                name: index.to_string().into(),
                path: format!("/{index}").into(),
                kind: EntryKind::File,
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

    #[test]
    fn filters_by_name_and_visibility_without_reordering() {
        let directory = Directory {
            entries: ["照片", ".Photo.jpg", "Photo 1.png", "photo 2.png"]
                .into_iter()
                .map(|name| Entry {
                    name: name.into(),
                    path: name.into(),
                    kind: EntryKind::File,
                })
                .collect(),
            read_errors: 0,
        };
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            filtered_entries(&directory, "", false, &cancelled),
            [0, 2, 3]
        );
        assert_eq!(
            filtered_entries(&directory, "photo", false, &cancelled),
            [2, 3]
        );
        assert_eq!(
            filtered_entries(&directory, "photo", true, &cancelled),
            [1, 2, 3]
        );
        cancelled.store(true, Relaxed);
        assert!(filtered_entries(&directory, "", true, &cancelled).is_empty());
    }
}
