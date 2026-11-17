use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

use gpui_kit::*;
use settings::config::tab::Location;
use vfs::{Directory, DirectoryWatch, EntryKind, read_directory, read_references};

use super::App;

pub(super) enum DirectoryState {
    Unavailable,
    Loading,
    Ready(Arc<DirectorySnapshot>),
    Failed(String),
}

pub(super) struct DirectorySnapshot {
    pub(super) directory: Directory,
    pub(super) paths: Option<Vec<PathBuf>>,
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

pub(super) struct Browser {
    pub(super) state: DirectoryState,
    pub(super) query: String,
    pub(super) show_hidden: bool,
    pub(super) visible: Vec<usize>,
    pub(super) selection: BTreeSet<usize>,
    pub(super) anchor: Option<usize>,
    pub(super) reveal_path: Option<PathBuf>,
    pub(super) reveal_row: Option<usize>,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) action_error: Option<String>,
    pub(super) filtering: bool,
    pub(super) snapshot_revision: u64,
    read_task: Option<ReadTask>,
    directory_watch: Option<Task<()>>,
    refresh_pending: bool,
    filter_task: Option<ReadTask>,
}

impl Browser {
    pub(super) fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
        self.reveal_path = None;
        self.reveal_row = None;
    }

    fn select_entry(&mut self, index: usize, modifiers: gpui_kit::Modifiers) {
        self.reveal_path = None;
        self.reveal_row = None;
        if modifiers.shift
            && let (Some(start), Ok(end)) = (
                self.anchor
                    .and_then(|anchor| self.visible.binary_search(&anchor).ok()),
                self.visible.binary_search(&index),
            )
        {
            if !modifiers.control {
                self.selection.clear();
            }
            self.selection
                .extend(&self.visible[start.min(end)..=start.max(end)]);
            return;
        }
        if modifiers.control {
            if !self.selection.remove(&index) {
                self.selection.insert(index);
            }
        } else {
            self.selection.clear();
            self.selection.insert(index);
        }
        self.anchor = Some(index);
    }

    pub(super) fn select_for_context_menu(&mut self, index: usize) -> bool {
        if self.selection.contains(&index) {
            return false;
        }
        self.select_entry(index, gpui_kit::Modifiers::default());
        true
    }

    pub(super) fn new(show_hidden: bool) -> Self {
        Self {
            state: DirectoryState::Unavailable,
            query: String::new(),
            show_hidden,
            visible: Vec::new(),
            selection: BTreeSet::new(),
            anchor: None,
            reveal_path: None,
            reveal_row: None,
            scroll: UniformListScrollHandle::new(),
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
        let source = self.current_location().map(|location| match location {
            Location::Directory { path } => ReadSource::Directory(path.0.clone()),
            Location::Tag { id } => self
                .tags(cx)
                .get(*id)
                .map_or(ReadSource::MissingTag, |tag| {
                    ReadSource::Tag(tag.paths.iter().map(|path| path.0.clone()).collect())
                }),
        });
        let tag_pending = matches!(source, Some(ReadSource::MissingTag))
            .then(|| self.tag_state.read(cx))
            .filter(|state| state.loaded_tags().is_none())
            .map(|state| match state.load_error() {
                Some(error) => DirectoryState::Failed(error.to_owned()),
                None => DirectoryState::Loading,
            });
        let browser = &mut self.browser;
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
        self.read_snapshot(source, false, cx);
    }

    fn watch_directory(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let watch = match DirectoryWatch::new(path) {
            Ok(watch) => watch,
            Err(error) => {
                eprintln!("无法监听目录 {}：{error}", path.display());
                return;
            }
        };
        let executor = cx.background_executor().clone();
        self.browser.directory_watch = Some(cx.spawn(async move |this, cx| {
            while watch.changed().await {
                // 合并复制、解压等短时间内连续产生的文件系统事件。
                executor.timer(Duration::from_millis(120)).await;
                watch.coalesce();
                if this
                    .update(cx, |this, cx| this.refresh_directory(cx))
                    .is_err()
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
        self.browser.read_task = None;
        self.browser.refresh_pending = false;
        self.browser.reveal_path = reveal;
        self.refresh_directory(cx);
    }

    fn refresh_directory(&mut self, cx: &mut Context<Self>) {
        if self.browser.read_task.is_some() {
            self.browser.refresh_pending = true;
            return;
        }
        let source = match self.current_location() {
            Some(Location::Directory { path }) => ReadSource::Directory(path.0.clone()),
            Some(Location::Tag { id }) => {
                let Some(tag) = self.tags(cx).get(*id) else {
                    return;
                };
                ReadSource::Tag(tag.paths.iter().map(|path| path.0.clone()).collect())
            }
            None => return,
        };
        self.read_snapshot(source, true, cx);
    }

    fn read_snapshot(&mut self, source: ReadSource, refresh: bool, cx: &mut Context<Self>) {
        let browser = &mut self.browser;
        let snapshot_revision = browser.snapshot_revision;
        let cancelled = Arc::new(AtomicBool::new(false));
        let read_cancelled = cancelled.clone();
        let query = browser.query.clone();
        let show_hidden = browser.show_hidden;
        let read = cx.background_spawn(async move {
            let is_cancelled = || read_cancelled.load(Relaxed);
            let (directory, paths) = match source {
                ReadSource::Directory(path) => (read_directory(&path, is_cancelled)?, None),
                ReadSource::Tag(paths) => {
                    let (directory, paths) = read_references(paths, is_cancelled)?;
                    (directory, Some(paths))
                }
                ReadSource::MissingTag => return Err(std::io::Error::other("标签已删除")),
            };
            let visible = filtered_entries(&directory, &query, show_hidden, &read_cancelled);
            Ok::<_, std::io::Error>((DirectorySnapshot { directory, paths }, visible, query))
        });
        let task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.browser.snapshot_revision != snapshot_revision {
                    return;
                }
                this.browser.read_task = None;
                let mut refresh_pending = std::mem::take(&mut this.browser.refresh_pending);
                match result {
                    Ok((directory, visible, query)) => {
                        let filter_matches =
                            query == this.browser.query && show_hidden == this.browser.show_hidden;
                        if filter_matches {
                            this.replace_snapshot(directory, visible);
                        } else if refresh {
                            // 旧索引仍对应当前快照，等后台按最新筛选条件读完再替换。
                            refresh_pending = true;
                        } else {
                            this.browser.state = DirectoryState::Ready(Arc::new(directory));
                            this.filter_directory(false, cx);
                        }
                    }
                    Err(error) => {
                        let browser = &mut this.browser;
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
        browser.read_task = Some(ReadTask {
            cancelled,
            _task: task,
        });
        cx.notify();
    }

    fn replace_snapshot(&mut self, snapshot: DirectorySnapshot, visible: Vec<usize>) {
        let browser = &mut self.browser;
        // 按排序键恢复选区。标签页可引用多个同名文件，因此也比较完整路径。
        if let DirectoryState::Ready(previous) = &browser.state
            && (!browser.selection.is_empty() || browser.anchor.is_some())
        {
            let tag_indices = snapshot.paths.as_ref().map(|paths| {
                paths
                    .iter()
                    .enumerate()
                    .map(|(index, path)| (path, index))
                    .collect::<HashMap<_, _>>()
            });
            let remap = |index: usize| {
                let new_index =
                    if let (Some(indices), Some(paths)) = (&tag_indices, &previous.paths) {
                        *indices.get(paths.get(index)?)?
                    } else {
                        let selected = previous.directory.entries.get(index)?;
                        let key = (selected.kind != EntryKind::Directory, &selected.name);
                        snapshot
                            .directory
                            .entries
                            .binary_search_by(|entry| {
                                (entry.kind != EntryKind::Directory, &entry.name).cmp(&key)
                            })
                            .ok()?
                    };
                visible.binary_search(&new_index).ok().map(|_| new_index)
            };
            browser.selection = browser
                .selection
                .iter()
                .filter_map(|&index| remap(index))
                .collect();
            browser.anchor = browser.anchor.and_then(remap);
        }
        if let Some(path) = browser.reveal_path.take()
            && let Some(position) = visible.iter().position(|&index| {
                if let Some(paths) = &snapshot.paths {
                    paths.get(index) == Some(&path)
                } else {
                    snapshot.directory.entries[index].name.as_os_str()
                        == path.file_name().unwrap_or_default()
                }
            })
        {
            let index = visible[position];
            browser.selection.clear();
            browser.selection.insert(index);
            browser.anchor = Some(index);
            browser.reveal_row = Some(position);
        }
        // 替换快照后，旧卡片和筛选任务不能继续使用原来的索引。
        browser.snapshot_revision += 1;
        browser.filter_task = None;
        browser.filtering = false;
        browser.state = DirectoryState::Ready(Arc::new(snapshot));
        browser.visible = visible;
    }

    pub(super) fn search_directory(&mut self, query: String, cx: &mut Context<Self>) {
        if self.browser.query != query {
            self.browser.query = query;
            self.filter_directory(true, cx);
        }
    }

    pub(super) fn set_show_hidden_files(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.browser.show_hidden != show {
            self.browser.show_hidden = show;
            self.filter_directory(false, cx);
            cx.notify();
        }
    }

    fn filter_directory(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let browser = &mut self.browser;
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
                if this.browser.snapshot_revision == snapshot_revision {
                    this.browser
                        .selection
                        .retain(|index| visible.binary_search(index).is_ok());
                    this.browser.anchor = this
                        .browser
                        .anchor
                        .filter(|index| visible.binary_search(index).is_ok());
                    this.browser.visible = visible;
                    this.browser.filtering = false;
                    this.browser.scroll = UniformListScrollHandle::new();
                    cx.notify();
                }
            });
        });
        browser.filter_task = Some(ReadTask {
            cancelled,
            _task: task,
        });
        cx.notify();
    }

    pub(super) fn entry_target(&self, index: usize) -> Option<(PathBuf, EntryKind)> {
        let DirectoryState::Ready(snapshot) = &self.browser.state else {
            return None;
        };
        let entry = snapshot.directory.entries.get(index)?;
        let path = match &snapshot.paths {
            Some(paths) => paths.get(index)?.clone(),
            None => self.location()?.join(&entry.name),
        };
        Some((path, entry.kind))
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
            self.browser.select_entry(index, modifiers);
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
        self.browser.select_entry(index, modifiers);
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
        let navigation_revision = self.navigation_revision;
        self.browser.action_error = None;
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
                if this.navigation_revision != navigation_revision {
                    return;
                }
                match result {
                    Ok(Some(path)) => this.navigate(path, window, cx),
                    Ok(None) => {}
                    Err(error) => {
                        this.browser.action_error = Some(format!("无法打开：{error}"));
                        cx.notify();
                    }
                }
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

    #[test]
    fn range_selection_respects_filter_and_control_click_toggles() {
        let mut browser = Browser::new(false);
        browser.visible = vec![0, 2, 5, 8];
        browser.select_entry(2, Modifiers::default());
        browser.select_entry(
            8,
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(
            browser.selection.iter().copied().collect::<Vec<_>>(),
            [2, 5, 8]
        );
        browser.select_entry(
            5,
            Modifiers {
                control: true,
                ..Default::default()
            },
        );
        assert_eq!(
            browser.selection.iter().copied().collect::<Vec<_>>(),
            [2, 8]
        );
        browser.clear_selection();
        assert!(browser.selection.is_empty());
        assert_eq!(browser.anchor, None);
    }

    #[test]
    fn context_menu_preserves_selected_group_and_replaces_it_for_other_entries() {
        let mut browser = Browser::new(false);
        browser.selection.extend([2, 5]);

        assert!(!browser.select_for_context_menu(5));
        assert_eq!(
            browser.selection.iter().copied().collect::<Vec<_>>(),
            [2, 5]
        );

        assert!(browser.select_for_context_menu(8));
        assert_eq!(browser.selection.iter().copied().collect::<Vec<_>>(), [8]);
        assert_eq!(browser.anchor, Some(8));
    }

    #[test]
    fn filters_by_name_and_visibility_without_reordering() {
        let directory = Directory {
            entries: ["照片", ".Photo.jpg", "Photo 1.png", "photo 2.png"]
                .into_iter()
                .map(|name| Entry {
                    name: name.into(),
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
