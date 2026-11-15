use std::sync::{Arc, Condvar, Mutex, PoisonError};

use gpui_kit::{AppContext, Context, Global};
use settings::config::{bookmark::Bookmarks, tab::Session, workspace::Workspace};

use super::App;

#[derive(Clone, Default)]
pub(super) struct WorkspaceWriter(Arc<Writer>);

impl Global for WorkspaceWriter {}

#[derive(Default)]
struct Writer {
    queue: Mutex<Queue>,
    idle: Condvar,
    disk: Mutex<()>,
}

#[derive(Default)]
struct Queue {
    pending: Option<Session>,
    running: bool,
    error: Option<String>,
}

impl WorkspaceWriter {
    pub(super) fn save_bookmarks(&self, bookmarks: Bookmarks) -> settings::Result<Bookmarks> {
        self.update_disk(move |workspace| workspace.bookmarks = bookmarks)
            .map(|workspace| workspace.bookmarks)
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
        let mut queue = self.0.queue.lock().unwrap_or_else(PoisonError::into_inner);
        // 写盘期间只留下最新的会话快照。
        queue.pending = Some(session);
        !std::mem::replace(&mut queue.running, true)
    }

    pub(super) fn session_error(&self) -> Option<String> {
        self.0
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .error
            .clone()
    }

    pub(super) fn flush(&self) {
        // 退出时等待最后一个快照落盘，后台线程不依赖 GPUI 主线程。
        let queue = self.0.queue.lock().unwrap_or_else(PoisonError::into_inner);
        drop(
            self.0
                .idle
                .wait_while(queue, |queue| queue.running)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }

    fn drain(&self) {
        loop {
            let session = {
                let mut queue = self.0.queue.lock().unwrap_or_else(PoisonError::into_inner);
                let Some(session) = queue.pending.take() else {
                    queue.running = false;
                    self.0.idle.notify_all();
                    return;
                };
                session
            };
            let error = self
                .update_disk(move |workspace| workspace.session = session)
                .err()
                .map(|error| format!("保存标签页会话失败：{error}"));
            if let Some(error) = &error {
                eprintln!("{error}");
            }
            self.0
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .error = error;
        }
    }

    fn update_disk(&self, change: impl FnOnce(&mut Workspace)) -> settings::Result<Workspace> {
        // 会话与书签共用同一个文件，读取到写回的全过程必须串行。
        let _disk = self
            .0
            .disk
            .lock()
            .map_err(|_| std::io::Error::other("工作区写入锁已损坏"))?;
        let mut workspace = Workspace::load()?;
        change(&mut workspace);
        workspace.save()?;
        Ok(workspace)
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
