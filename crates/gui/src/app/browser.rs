use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

use gpui_kit::*;
use settings::config::tab::Location;
use vfs::{Directory, EntryKind, read_directory, read_references};

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
    pub(super) selected_entry: Option<usize>,
    pub(super) scroll: UniformListScrollHandle,
    pub(super) open_error: Option<String>,
    pub(super) filtering: bool,
    pub(super) generation: u64,
    read_task: Option<ReadTask>,
    filter_task: Option<ReadTask>,
}

impl Browser {
    pub(super) fn new(show_hidden: bool) -> Self {
        Self {
            state: DirectoryState::Unavailable,
            query: String::new(),
            show_hidden,
            visible: Vec::new(),
            selected_entry: None,
            scroll: UniformListScrollHandle::new(),
            open_error: None,
            filtering: false,
            generation: 0,
            read_task: None,
            filter_task: None,
        }
    }
}

impl App {
    pub(super) fn load_directory(&mut self, cx: &mut Context<Self>) {
        self.file_item_menu.clear();
        let source = self.current_location().map(|location| match location {
            Location::Directory { path } => ReadSource::Directory(path.0.clone()),
            Location::Tag { id } => self.tags.get(*id).map_or(ReadSource::MissingTag, |tag| {
                ReadSource::Tag(tag.paths.iter().map(|path| path.0.clone()).collect())
            }),
        });
        let browser = &mut self.browser;
        browser.generation += 1;
        browser.read_task = None;
        browser.filter_task = None;
        browser.filtering = false;
        browser.visible = Vec::new();
        browser.selected_entry = None;
        browser.open_error = None;
        browser.scroll = UniformListScrollHandle::new();
        let Some(source) = source else {
            browser.state = DirectoryState::Unavailable;
            cx.notify();
            return;
        };

        browser.state = DirectoryState::Loading;
        let generation = browser.generation;
        let cancelled = Arc::new(AtomicBool::new(false));
        let read_cancelled = cancelled.clone();
        let query = browser.query.clone();
        let show_hidden = browser.show_hidden;
        let initial_query = query.clone();
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
            let visible =
                filtered_entries(&directory, &initial_query, show_hidden, &read_cancelled);
            Ok::<_, std::io::Error>((DirectorySnapshot { directory, paths }, visible))
        });
        let task = cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.browser.generation != generation {
                    return;
                }
                match result {
                    Ok((directory, visible)) => {
                        this.browser.state = DirectoryState::Ready(Arc::new(directory));
                        if query == this.browser.query && show_hidden == this.browser.show_hidden {
                            this.browser.visible = visible;
                        } else {
                            this.filter_directory(false, cx);
                        }
                    }
                    Err(error) => this.browser.state = DirectoryState::Failed(error.to_string()),
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
        let generation = browser.generation;
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
                if this.browser.generation == generation {
                    if this
                        .browser
                        .selected_entry
                        .is_some_and(|index| !visible.contains(&index))
                    {
                        this.browser.selected_entry = None;
                    }
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, kind)) = self.entry_target(index) else {
            return;
        };
        if kind == EntryKind::Directory {
            if click_count == 1 {
                self.navigate(path, window, cx);
            }
            return;
        }
        if self.browser.selected_entry != Some(index) {
            self.browser.selected_entry = Some(index);
            cx.notify();
        }
        if click_count != 2 || !matches!(kind, EntryKind::File | EntryKind::Symlink) {
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
        let generation = self.browser.generation;
        self.browser.open_error = None;
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
                if this.browser.generation != generation {
                    return;
                }
                match result {
                    Ok(Some(path)) => this.navigate(path, window, cx),
                    Ok(None) => {}
                    Err(error) => {
                        this.browser.open_error = Some(format!("无法打开：{error}"));
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
    use super::{AtomicBool, Relaxed, filtered_entries};
    use vfs::{Directory, Entry, EntryKind};

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
