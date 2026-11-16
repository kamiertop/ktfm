//! 外部终端：快捷键（ctrl-`）与右键菜单的入口，经 [`sys::opener`]
//! 启动用户配置的终端程序（支持带参数，留空自动探测）。

use std::path::PathBuf;

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{App, preferences::ActivePreferences};

actions!(browser, [OpenTerminal]);

impl App {
    /// 快捷键入口：在当前目录打开外部终端。
    /// 内嵌终端已移除，待重新实现后此处再按配置分派。
    pub(super) fn toggle_terminal(&mut self, cx: &mut Context<Self>) {
        // 启动外部进程不与界面模态（重命名、属性）争抢焦点，无需守卫。
        if let Some(directory) = self.location().map(ToOwned::to_owned) {
            self.open_external_terminal(directory, cx);
        }
    }

    /// 在指定目录启动外部终端。启动失败时把错误报给当前目录的错误条，
    /// 用户离开该目录后过期的错误不再显示。
    pub(super) fn open_external_terminal(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        let program = cx
            .global::<ActivePreferences>()
            .0
            .terminal
            .external_program
            .clone();
        self.browser.action_error = None;
        let revision = self.navigation_revision;
        let launch =
            cx.background_spawn(async move { sys::opener::open_terminal(&directory, &program) });
        cx.spawn(async move |this, cx| {
            match launch.await {
                Ok(mut child) => {
                    // 等待子进程退出，避免留下僵尸进程。
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.navigation_revision == revision {
                            this.browser.action_error = Some(format!("无法启动外部终端：{error}"));
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
        cx.notify();
    }
}
