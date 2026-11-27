//! 目录快照、当前目录筛选和排序。
//! 后续在 daemon 中接入 ripgrep（rg）做递归内容搜索。
//! 当前目录的文件名筛选继续使用已有快照。

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;

use vfs::Directory;

mod counts;
mod sort;

pub use counts::{ChildCount, ChildCountMonitor, ChildCountUpdate};
pub use sort::{FileSort, SortColumn, SortDirection, type_sort_key};

/// 一次浏览读取的结果，以及按路径定位条目的索引。
pub struct DirectorySnapshot {
    pub directory: Directory,
    pub indices: HashMap<PathBuf, usize>,
}

impl DirectorySnapshot {
    /// 读取目录或标签引用。取消时返回 `Interrupted`。
    pub fn read(source: ReadSource, cancelled: &AtomicBool) -> io::Result<Self> {
        let is_cancelled = || cancelled.load(Relaxed);
        let directory = match source {
            ReadSource::Directory(path) => vfs::read_directory(&path, is_cancelled)?,
            ReadSource::Tag(paths) => vfs::read_references(paths, is_cancelled)?,
            ReadSource::MissingTag => return Err(io::Error::other("标签已删除")),
        };
        Ok(Self::new(directory))
    }

    /// 返回符合筛选条件的条目索引。未指定排序时保留快照顺序。
    #[must_use]
    pub fn visible_entries(
        &self,
        query: &str,
        show_hidden: bool,
        sort: FileSort,
        counts: &HashMap<usize, ChildCount>,
        cancelled: &AtomicBool,
    ) -> Vec<usize> {
        let mut visible = filtered_entries(&self.directory, query, show_hidden, cancelled);
        if !sort.is_default() && !cancelled.load(Relaxed) {
            sort.order(&self.directory.entries, &mut visible, |index| {
                counts.get(&index).and_then(|entry| entry.count)
            });
        }
        visible
    }

    /// 为已有目录结果建立路径索引。
    #[must_use]
    pub fn new(directory: Directory) -> Self {
        let indices = directory
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect();
        Self { directory, indices }
    }
}

/// 目录快照的读取来源，包括已失效的标签。
pub enum ReadSource {
    Directory(PathBuf),
    Tag(Vec<PathBuf>),
    MissingTag,
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
    use super::*;
    use vfs::{Entry, EntryKind};
    #[test]
    fn filters_by_name_and_visibility_without_reordering() {
        let directory = Directory {
            entries: ["照片", ".Photo.jpg", "Photo 1.png", "photo 2.png"]
                .into_iter()
                .map(|name| Entry {
                    name: name.into(),
                    path: name.into(),
                    kind: EntryKind::File,
                    size: None,
                    modified: None,
                    created: None,
                    accessed: None,
                    access: None,
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
