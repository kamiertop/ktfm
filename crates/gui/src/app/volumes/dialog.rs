use std::io;

use gpui_kit::base::{Dialog, Disableable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use sys::mount::{Volume, VolumeSource};
use zeroize::Zeroizing;

use super::{App, OpenVolumeTarget};
use crate::app::file_area::format_size;
use crate::components::popup_surface;

pub(in crate::app) struct VolumeUnlock {
    source: VolumeSource,
    label: String,
    target: OpenVolumeTarget,
    input: Entity<InputState>,
    error: Option<String>,
    running: bool,
    focus: FocusHandle,
}

impl App {
    pub(super) fn show_volume_unlock(
        &mut self,
        volume: Volume,
        target: OpenVolumeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("密码"));
        input.update(cx, |input, cx| input.focus(window, cx));
        self.volume_unlock = Some(VolumeUnlock {
            source: volume.source,
            label: volume.label,
            target,
            input,
            error: None,
            running: false,
            focus: cx.focus_handle(),
        });
        cx.notify();
    }

    fn submit_volume_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(unlock) = &mut self.volume_unlock else {
            return;
        };
        if unlock.running || self.storage.read(cx).operation.is_some() {
            return;
        }
        let password = Zeroizing::new(unlock.input.read(cx).text().to_string());
        if password.is_empty() {
            unlock.error = Some("请输入密码".into());
            cx.notify();
            return;
        }
        let source = unlock.source.clone();
        let target = unlock.target;
        let Some(volume) = self.storage.read(cx).volume(&source) else {
            unlock.error = Some("设备已断开".into());
            cx.notify();
            return;
        };
        if !volume.locked {
            self.close_volume_unlock(window, cx);
            self.open_volume_at(&source, target, window, cx);
            return;
        }
        unlock
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        unlock.error = None;
        unlock.running = true;
        self.mount_volume(source, target, Some(password), window, cx);
        cx.notify();
    }

    pub(super) fn show_volume_unlock_error(
        &mut self,
        error: &io::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(unlock) = &mut self.volume_unlock {
            unlock.running = false;
            unlock.error = Some(format!("无法解锁：{error}"));
            unlock.input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn close_volume_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.volume_unlock.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(in crate::app) fn volume_unlock_dialog(&self, cx: &mut Context<Self>) -> Option<Dialog> {
        let unlock = self.volume_unlock.as_ref()?;
        let running = unlock.running;
        let busy = self.storage.read(cx).operation.is_some();
        let danger = cx.theme().danger;
        let owner = cx.weak_entity();
        let confirm = owner.clone();
        Some(
            Dialog::new(cx)
                .focus_handle(unlock.focus.clone())
                .accessibility_label("解锁存储设备")
                .on_ok(move |_, window, cx| {
                    let _ = confirm.update(cx, |this, cx| this.submit_volume_unlock(window, cx));
                    false
                })
                .on_open_change(move |open, _, window, cx| {
                    if !open && !running {
                        let _ = owner.update(cx, |this, cx| this.close_volume_unlock(window, cx));
                    }
                })
                .backdrop(div().size_full().bg(black().opacity(0.35)).occlude())
                .popup(
                    popup_surface("volume-unlock", cx)
                        .w(px(380.))
                        .max_w(relative(0.9))
                        .p_5()
                        .occlude()
                        .child(
                            v_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("解锁存储设备"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .min_w_0()
                                        .truncate()
                                        .child(unlock.label.clone()),
                                )
                                .child(Input::new(&unlock.input).disabled(running))
                                .when_some(unlock.error.clone(), |panel, error| {
                                    panel.child(div().text_sm().text_color(danger).child(error))
                                })
                                .child(
                                    h_flex()
                                        .justify_end()
                                        .gap_2()
                                        .child(
                                            Button::new("cancel-volume-unlock")
                                                .ghost()
                                                .label("取消")
                                                .disabled(running)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.close_volume_unlock(window, cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("submit-volume-unlock")
                                                .primary()
                                                .label(if running {
                                                    "正在解锁…"
                                                } else {
                                                    "解锁并挂载"
                                                })
                                                .disabled(busy)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_volume_unlock(window, cx)
                                                })),
                                        ),
                                ),
                        ),
                ),
        )
    }
}

pub(in crate::app) fn volume_details(details: Div, volume: &Volume) -> Div {
    let status = if volume.locked {
        "已锁定"
    } else if volume.mounted {
        "已挂载"
    } else {
        "未挂载"
    };
    details
        .child(format!("名称　{}", volume.label))
        .child(format!("设备　{}", volume.source))
        .child(format!("状态　{status}"))
        .when_some(volume.mountpoint.as_ref(), |details, path| {
            details.child(format!("挂载点　{}", path.display()))
        })
        .when_some(
            volume.usage.map(|usage| usage.total).or(volume.capacity),
            |details, total| details.child(format!("容量　{}", format_size(total))),
        )
        .when_some(volume.usage, |details, usage| {
            details.child(format!("已用　{}", format_size(usage.used)))
        })
}
