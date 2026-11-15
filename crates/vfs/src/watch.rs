use std::path::Path;

use async_channel::{Receiver, TryRecvError};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// 监听目录的直接子项。只保留一个待处理通知，避免批量操作堆积事件。
pub struct DirectoryWatch {
    _watcher: RecommendedWatcher,
    changes: Receiver<()>,
}

impl DirectoryWatch {
    pub fn new(path: &Path) -> notify::Result<Self> {
        let (sender, changes) = async_channel::bounded(1);
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let refresh = match event {
                Ok(event) => {
                    event.need_rescan()
                        || !matches!(
                            event.kind,
                            EventKind::Access(_)
                                | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_))
                        )
                }
                Err(error) => {
                    eprintln!("目录监听出错：{error}");
                    true
                }
            };
            if refresh {
                // 队列已满说明已有刷新待处理，关闭则表示界面已离开目录。
                let _ = sender.try_send(());
            }
        })?;
        watcher.watch(path, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _watcher: watcher,
            changes,
        })
    }

    pub async fn changed(&self) -> bool {
        self.changes.recv().await.is_ok()
    }

    pub fn coalesce(&self) {
        match self.changes.try_recv() {
            Ok(()) | Err(TryRecvError::Empty | TryRecvError::Closed) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    use super::*;

    #[test]
    fn detects_external_creation() -> Result<(), Box<dyn std::error::Error>> {
        let path =
            std::env::temp_dir().join(format!("ktfm-test-directory-watch-{}", std::process::id()));
        fs::create_dir(&path)?;
        let result = (|| {
            let watch = DirectoryWatch::new(&path)?;
            fs::write(path.join("new-file"), "")?;
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match watch.changes.try_recv() {
                    Ok(()) => return Ok(()),
                    Err(TryRecvError::Empty) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        })();
        fs::remove_dir_all(path)?;
        result
    }
}
