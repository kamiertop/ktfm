//! 记住普通窗口的大小，并在下次打开窗口时恢复。

use std::time::Duration;

use gpui_kit::*;
use settings::config::workspace::{WindowSize, Workspace};

use super::{App, io_queue::IoQueue};
use crate::style::{MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH, WINDOW_HEIGHT, WINDOW_WIDTH};

const SAVE_DELAY: Duration = Duration::from_millis(400);

pub(super) fn startup_bounds(saved: Option<WindowSize>, cx: &gpui_kit::App) -> WindowBounds {
    let size = saved
        .and_then(WindowSize::sanitize)
        .map(|saved| {
            size(
                px(saved.width).max(MIN_WINDOW_WIDTH),
                px(saved.height).max(MIN_WINDOW_HEIGHT),
            )
        })
        .unwrap_or_else(|| size(WINDOW_WIDTH, WINDOW_HEIGHT));
    WindowBounds::centered(size, cx)
}

fn windowed_size(window: &Window) -> Option<WindowSize> {
    if window.is_maximized() || window.is_fullscreen() {
        return None;
    }
    let bounds = window.bounds().size;
    WindowSize {
        width: bounds.width.as_f32(),
        height: bounds.height.as_f32(),
    }
    .sanitize()
}

#[derive(Default)]
pub(super) struct GeometrySaver {
    pending: Option<WindowSize>,
    timer: Option<Task<()>>,
}

impl Global for GeometrySaver {}

impl GeometrySaver {
    pub(super) fn observe(window: &mut Window, cx: &mut Context<App>) {
        let mut last = windowed_size(window);
        cx.observe_window_bounds(window, move |_, window, cx| {
            let Some(size) = windowed_size(window) else {
                return;
            };
            if last != Some(size) {
                last = Some(size);
                Self::schedule(size, cx);
            }
        })
        .detach();
    }

    fn schedule(size: WindowSize, cx: &mut gpui_kit::App) {
        let executor = cx.background_executor().clone();
        let timer = cx.spawn(async move |cx| {
            executor.timer(SAVE_DELAY).await;
            cx.update(Self::submit_pending);
        });
        // 多个窗口共用一个待保存尺寸。单个窗口关闭后，计时器仍会提交。
        let saver = cx.global_mut::<Self>();
        saver.pending = Some(size);
        saver.timer = Some(timer);
    }

    pub(super) fn flush(cx: &mut gpui_kit::App) {
        cx.global_mut::<Self>().timer.take();
        Self::submit_pending(cx);
    }

    fn submit_pending(cx: &mut gpui_kit::App) {
        let Some(size) = cx.global_mut::<Self>().pending.take() else {
            return;
        };
        let queued = cx.global::<IoQueue>().enqueue(move || {
            if let Err(error) = Workspace::update(|workspace| workspace.window_size = Some(size)) {
                tracing::warn!(%error, "无法保存窗口大小");
            }
        });
        if !queued {
            tracing::warn!("存储线程已停止，无法保存窗口大小");
        }
    }
}
