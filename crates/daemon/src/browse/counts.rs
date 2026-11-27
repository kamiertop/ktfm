use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, SystemTime};

use async_io::Timer;
use vfs::DirectoryWatch;

use super::{DirectorySnapshot, FileSort};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ChildCount {
    modified: Option<SystemTime>,
    pub count: Option<u64>,
}

pub struct ChildCountUpdate {
    pub changed: Vec<(usize, ChildCount)>,
    pub reordered: Option<Vec<usize>>,
}

/// 同一份目录快照内维护计数缓存，范围由调用者根据可见行或排序方式决定。
pub struct ChildCountMonitor {
    snapshot: Arc<DirectorySnapshot>,
    targets: Vec<usize>,
    counts: HashMap<usize, ChildCount>,
    visible: Option<Vec<usize>>,
    sort: FileSort,
    watch: Option<DirectoryWatch>,
}

impl ChildCountMonitor {
    pub fn new(
        snapshot: Arc<DirectorySnapshot>,
        targets: Vec<usize>,
        counts: HashMap<usize, ChildCount>,
        visible: Option<Vec<usize>>,
        sort: FileSort,
        cancelled: &AtomicBool,
    ) -> Self {
        // 先注册，再读取，覆盖统计期间发生的目录变更。
        let watch = DirectoryWatch::for_directories(
            targets
                .iter()
                .take_while(|_| !cancelled.load(Relaxed))
                .map(|&index| snapshot.directory.entries[index].path.as_path()),
        )
        .ok();
        Self {
            snapshot,
            targets,
            counts,
            visible,
            sort,
            watch,
        }
    }

    pub fn refresh(&mut self, cancelled: &AtomicBool) -> ChildCountUpdate {
        let changed =
            refresh_child_counts(&self.snapshot, &self.targets, &mut self.counts, cancelled);
        let reordered = self
            .visible
            .as_ref()
            .filter(|_| !changed.is_empty() && !cancelled.load(Relaxed))
            .map(|visible| {
                let mut visible = visible.clone();
                self.sort
                    .order(&self.snapshot.directory.entries, &mut visible, |index| {
                        self.counts.get(&index).and_then(|entry| entry.count)
                    });
                visible
            });
        if self.counts.values().any(|entry| entry.modified.is_none()) {
            self.watch = None;
        }
        ChildCountUpdate { changed, reordered }
    }

    pub async fn wait_for_change(&self) -> bool {
        if let Some(watch) = &self.watch {
            if !watch.changed().await {
                return false;
            }
            Timer::after(Duration::from_millis(120)).await;
            watch.coalesce();
        } else {
            // 监听不可用时低频检查时间戳，让重新出现的目录也能恢复计数。
            Timer::after(Duration::from_secs(3)).await;
        }
        true
    }
}

fn refresh_child_counts(
    snapshot: &DirectorySnapshot,
    targets: &[usize],
    counts: &mut HashMap<usize, ChildCount>,
    cancelled: &AtomicBool,
) -> Vec<(usize, ChildCount)> {
    let mut changed = Vec::new();
    for &index in targets {
        if cancelled.load(Relaxed) {
            break;
        }
        let path = &snapshot.directory.entries[index].path;
        let metadata = std::fs::symlink_metadata(path)
            .ok()
            .filter(|metadata| metadata.is_dir());
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok());
        // 未变化的目录只读取时间戳，避免重新枚举全部项目。
        if modified.is_some()
            && counts
                .get(&index)
                .is_some_and(|old| old.modified == modified && old.count.is_some())
        {
            continue;
        }
        let count = if metadata.is_some() {
            let Ok(count) = vfs::read_child_count(path, || cancelled.load(Relaxed)) else {
                break;
            };
            count
        } else {
            None
        };
        let next = ChildCount { modified, count };
        if counts.insert(index, next) != Some(next) {
            changed.push((index, next));
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_count_cache_reuses_fresh_values_and_recounts_stale_ones() -> std::io::Result<()> {
        use std::fs;
        use std::time::SystemTime;

        let root = tempfile::tempdir()?;
        let directory = root.path().join("directory");
        fs::create_dir(&directory)?;
        let snapshot = DirectorySnapshot::new(vfs::read_directory(root.path(), || false)?);
        let cancelled = AtomicBool::new(false);
        let mut counts = HashMap::new();
        let first = refresh_child_counts(&snapshot, &[0], &mut counts, &cancelled);
        assert_eq!(first[0].1.count, Some(0));
        assert!(refresh_child_counts(&snapshot, &[0], &mut counts, &cancelled).is_empty());

        fs::write(directory.join("entry"), "")?;
        counts.get_mut(&0).unwrap().modified = Some(SystemTime::UNIX_EPOCH);
        let next = refresh_child_counts(&snapshot, &[0], &mut counts, &cancelled);
        assert_eq!(next[0].1.count, Some(1));
        Ok(())
    }
}
