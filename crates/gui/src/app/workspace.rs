use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use gpui_kit::{AppContext, Context, Entity, Global};
use settings::config::bookmark::Bookmarks;
use settings::config::tab::Session;
use settings::config::workspace::Workspace;

use vfs::DirectoryWatch;

use super::App;

#[derive(Clone, Default)]
pub(super) struct WorkspaceWriter(Arc<Writer>);

impl Global for WorkspaceWriter {}

#[derive(Default)]
struct Writer {
    queue: Mutex<Queue>,
    idle: Condvar,
    bookmark_revision: AtomicU64,
    bookmark_writes: AtomicU64,
}

#[derive(Default)]
struct Queue {
    pending: Option<Session>,
    running: bool,
    error: Option<String>,
}

impl WorkspaceWriter {
    pub(super) fn change_bookmarks(
        &self,
        change: impl FnOnce(&mut Bookmarks),
    ) -> settings::Result<Bookmarks> {
        Workspace::update(|workspace| change(&mut workspace.bookmarks))
            .map(|workspace| workspace.bookmarks)
    }

    pub(super) fn begin_bookmark_update(&self) {
        self.0.bookmark_writes.fetch_add(1, Relaxed);
    }

    pub(super) fn finish_bookmark_update(&self) {
        self.0.bookmark_writes.fetch_sub(1, Relaxed);
        self.0.bookmark_revision.fetch_add(1, Relaxed);
    }

    pub(super) fn watch_bookmarks(&self, bookmarks: &Entity<Bookmarks>, cx: &mut Context<App>) {
        let watch = Workspace::path()
            .map_err(anyhow::Error::from)
            .and_then(|path| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Ok(DirectoryWatch::for_file(&path)?)
            });
        let watch = match watch {
            Ok(watch) => watch,
            Err(error) => {
                tracing::warn!(%error, "无法监听书签更新");
                return;
            }
        };
        let writer = self.clone();
        let bookmarks = bookmarks.downgrade();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |_, cx| {
            // 注册后先补读一次，覆盖启动时读取和注册监听之间的变更。
            loop {
                let revision = writer.0.bookmark_revision.load(Relaxed);
                if writer.0.bookmark_writes.load(Relaxed) != 0 {
                    executor.timer(Duration::from_millis(20)).await;
                    continue;
                }
                let result = executor.spawn(async { Workspace::load() }).await;
                let updated = cx.update(|cx| {
                    bookmarks.update(cx, |bookmarks, cx| {
                        // 后台读取期间若本进程完成了写入，丢弃旧快照并重读。
                        if writer.0.bookmark_revision.load(Relaxed) != revision
                            || writer.0.bookmark_writes.load(Relaxed) != 0
                        {
                            return false;
                        }
                        match result {
                            Ok(workspace) if *bookmarks != workspace.bookmarks => {
                                *bookmarks = workspace.bookmarks;
                                cx.notify();
                            }
                            Err(error) => tracing::warn!(%error, "无法同步书签"),
                            _ => {}
                        }
                        true
                    })
                });
                match updated {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(_) => break,
                }
                if !watch.changed().await {
                    break;
                }
                executor.timer(Duration::from_millis(80)).await;
                watch.coalesce();
            }
        })
        .detach();
    }

    pub(super) fn save_session(&self, session: Session, cx: &mut Context<App>) {
        if !self.enqueue_session(session) {
            return;
        }
        let writer = self.clone();
        let save = cx.background_spawn(async move { writer.drain() });
        cx.spawn(async move |view, cx| {
            save.await;
            let _ = view.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn enqueue_session(&self, session: Session) -> bool {
        let mut queue = self.0.queue.lock().expect("会话保存队列的互斥锁不应中毒");
        // 写盘期间只留下最新的会话快照。
        queue.pending = Some(session);
        !std::mem::replace(&mut queue.running, true)
    }

    pub(super) fn session_error(&self) -> Option<String> {
        self.0
            .queue
            .lock()
            .expect("会话保存队列的互斥锁不应中毒")
            .error
            .clone()
    }

    pub(super) fn flush(&self) {
        // 退出时等待最后一个快照落盘，后台线程不依赖 GPUI 主线程。
        let queue = self.0.queue.lock().expect("会话保存队列的互斥锁不应中毒");
        drop(
            self.0
                .idle
                .wait_while(queue, |queue| queue.running)
                .expect("会话保存队列的互斥锁不应中毒"),
        );
    }

    fn drain(&self) {
        loop {
            let session = {
                let mut queue = self.0.queue.lock().expect("会话保存队列的互斥锁不应中毒");
                let Some(session) = queue.pending.take() else {
                    queue.running = false;
                    self.0.idle.notify_all();
                    return;
                };
                session
            };
            let error = Workspace::update(move |workspace| workspace.session = session)
                .err()
                .map(|error| format!("保存标签页会话失败：{error}"));
            if let Some(error) = &error {
                tracing::error!("{error}");
            }
            self.0
                .queue
                .lock()
                .expect("会话保存队列的互斥锁不应中毒")
                .error = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn pending_session_replaces_older_snapshots_without_starting_another_writer() {
        let writer = WorkspaceWriter::default();
        let session = |id| Session {
            tabs: Vec::new(),
            active: Some(id),
            ..Session::default()
        };
        let first = Uuid::new_v4();
        let latest = Uuid::new_v4();
        assert!(writer.enqueue_session(session(first)));
        assert!(!writer.enqueue_session(session(latest)));

        let queue = writer.0.queue.lock().unwrap();
        assert_eq!(
            queue.pending.as_ref().and_then(|session| session.active),
            Some(latest)
        );
    }
}
