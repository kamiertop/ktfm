//! 按提交顺序执行持久化操作。写盘不依赖界面事件循环。

use std::cell::RefCell;
use std::io;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use gpui_kit::{App, Global, Task};

type Job = Box<dyn FnOnce() + Send>;

pub(super) struct IoQueue {
    sender: Option<Sender<Job>>,
    worker: Option<JoinHandle<()>>,
    callbacks: RefCell<Option<Task<()>>>,
}

impl Global for IoQueue {}

impl IoQueue {
    pub(super) fn new() -> io::Result<Self> {
        let (sender, jobs) = mpsc::channel::<Job>();
        let worker = thread::Builder::new()
            .name("ktfm-storage".into())
            .spawn(move || {
                for job in jobs {
                    job();
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            callbacks: RefCell::default(),
        })
    }

    pub(super) fn submit<R: Send + 'static>(
        cx: &mut App,
        work: impl FnOnce() -> settings::Result<R> + Send + 'static,
        complete: impl FnOnce(settings::Result<R>, &mut App) + 'static,
    ) {
        let (result, completion) = async_channel::bounded(1);
        let queue = cx.global::<Self>();
        if let Some(sender) = &queue.sender {
            let _ = sender.send(Box::new(move || {
                let _ = result.try_send(work());
            }));
        }
        let previous = queue.callbacks.take();
        let callback = cx.spawn(async move |cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = completion.recv().await.unwrap_or_else(|_| {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "存储线程已停止").into())
            });
            cx.update(|cx| complete(result, cx));
        });
        cx.global::<Self>().callbacks.replace(Some(callback));
    }

    pub(super) fn finish(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::error!("存储线程异常退出");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_drains_accepted_jobs_without_gui_callbacks() -> io::Result<()> {
        let mut queue = IoQueue::new()?;
        let (result, completed) = mpsc::channel();
        let sender = queue.sender.as_ref().expect("存储线程已启动");
        for index in 0..3 {
            let result = result.clone();
            sender
                .send(Box::new(move || {
                    let _ = result.send(index);
                }))
                .expect("提交成功");
        }
        queue.finish();
        assert_eq!(completed.try_iter().collect::<Vec<_>>(), [0, 1, 2]);
        Ok(())
    }
}
