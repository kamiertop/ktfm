use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Dialog, Disableable as _};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::tab::Location;
use sys::mount::{self, UnlockMountError, Volume, VolumeSource};
use zeroize::Zeroizing;

use crate::components::popup_surface;

use super::{App, menu::app_menu_item};

#[derive(Default)]
pub(super) struct StorageDevices {
    pub(super) volumes: Vec<Volume>,
    pub(super) operation: Option<VolumeOperation>,
    pub(super) error: Option<String>,
    pub(super) discovery_error: Option<String>,
    revision: u64,
    discovery_pending: bool,
}

pub(super) struct VolumeUnmounted(pub(super) PathBuf);

impl EventEmitter<VolumeUnmounted> for StorageDevices {}

pub(super) struct VolumeOperation {
    pub(super) source: VolumeSource,
    pub(super) unmounting: bool,
}

#[derive(Clone, Copy)]
enum OpenVolumeTarget {
    CurrentTab,
    NewTab,
    NewWindow,
}

pub(super) struct VolumeUnlock {
    volume: Volume,
    target: OpenVolumeTarget,
    input: Entity<InputState>,
    error: Option<String>,
    running: bool,
    focus: FocusHandle,
}

pub(super) struct SharedStorage {
    pub(super) state: Entity<StorageDevices>,
    discovery: async_channel::Sender<()>,
    usage: async_channel::Sender<()>,
}

impl Global for SharedStorage {}

impl StorageDevices {
    fn finish_operation(&mut self, cx: &mut Context<Self>) {
        self.operation = None;
        self.revision += 1;
        if std::mem::take(&mut self.discovery_pending) {
            let _ = cx.global::<SharedStorage>().discovery.try_send(());
        }
    }
}

impl SharedStorage {
    pub(super) fn install(cx: &mut gpui_kit::App) {
        let state = cx.new(|_| StorageDevices::default());
        let (discovery, discovery_requests) = async_channel::bounded(1);
        let (usage, usage_requests) = async_channel::bounded(1);
        cx.set_global(Self {
            state: state.clone(),
            discovery: discovery.clone(),
            usage,
        });

        // 在创建主窗口前提交首次扫描，让两者并行进行。
        let discovered = state.clone();
        let mut revision = state.read(cx).revision;
        let mut scan = cx.background_spawn(async { mount::volumes() });
        cx.spawn(async move |cx| {
            loop {
                let volumes = scan.await;
                let retry = cx.update(|cx| {
                    discovered.update(cx, |state, cx| {
                        if state.operation.is_some() {
                            state.discovery_pending = true;
                            return false;
                        }
                        if revision != state.revision {
                            return true;
                        }
                        let error = match volumes {
                            Ok(volumes) => {
                                if state.volumes != volumes {
                                    state.volumes = volumes;
                                    state.revision += 1;
                                    cx.notify();
                                }
                                None
                            }
                            Err(error) => Some(format!("无法读取设备：{error}")),
                        };
                        if state.discovery_error != error {
                            state.discovery_error = error;
                            cx.notify();
                        }
                        false
                    })
                });
                if !retry && discovery_requests.recv().await.is_err() {
                    break;
                }
                // 一次插拔可能产生多条卷和挂载事件，只扫描一次。
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let _ = discovery_requests.try_recv();
                revision = cx.update(|cx| discovered.read(cx).revision);
                scan = cx.background_spawn(async { mount::volumes() });
            }
        })
        .detach();

        #[cfg(target_os = "linux")]
        Self::watch_devices(discovery, cx);

        cx.spawn(async move |cx| {
            loop {
                let timer = cx.background_executor().timer(Duration::from_secs(30));
                if futures_lite::future::or(usage_requests.recv(), async {
                    timer.await;
                    Ok(())
                })
                .await
                .is_err()
                {
                    break;
                }
                let Some((revision, mut volumes)) = cx.update(|cx| {
                    let state = state.read(cx);
                    (state.operation.is_none()
                        && state
                            .volumes
                            .iter()
                            .any(|volume| volume.mountpoint.is_some()))
                    .then(|| (state.revision, state.volumes.clone()))
                }) else {
                    continue;
                };
                let volumes = cx
                    .background_spawn(async move {
                        mount::refresh_usage(&mut volumes);
                        volumes
                    })
                    .await;
                cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if state.revision == revision
                            && state.operation.is_none()
                            && state.volumes != volumes
                        {
                            state.volumes = volumes;
                            cx.notify();
                        }
                    })
                });
            }
        })
        .detach();
    }

    #[cfg(target_os = "linux")]
    fn watch_devices(discovery: async_channel::Sender<()>, cx: &mut gpui_kit::App) {
        let executor = cx.background_executor().clone();
        let (stop, stopped) = async_channel::bounded::<()>(1);
        let mut watcher = Some(cx.background_spawn(async move {
            futures_lite::future::or(
                async {
                    let _ = stopped.recv().await;
                },
                async {
                    loop {
                        if let Err(error) = mount::watch_volumes(|| {
                            let _ = discovery.try_send(());
                        })
                        .await
                        {
                            tracing::debug!(%error, "卷监视器停止，稍后重试");
                        }
                        let _ = discovery.try_send(());
                        executor.timer(Duration::from_secs(30)).await;
                    }
                },
            )
            .await;
        }));
        cx.on_app_quit(move |_| {
            stop.close();
            let watcher = watcher.take();
            async move {
                if let Some(watcher) = watcher {
                    watcher.await;
                }
            }
        })
        .detach();
    }

    pub(super) fn request_usage_refresh(cx: &gpui_kit::App) {
        let _ = cx.global::<Self>().usage.try_send(());
    }
}

impl App {
    pub(super) fn open_volume(
        &mut self,
        volume: &Volume,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_volume_at(volume, OpenVolumeTarget::CurrentTab, window, cx);
    }

    pub(super) fn build_volume_menu(
        &self,
        menu: PopupMenu,
        source: &VolumeSource,
        owner: &WeakEntity<Self>,
        cx: &gpui_kit::App,
    ) -> PopupMenu {
        let storage = self.storage.read(cx);
        let Some(volume) = storage
            .volumes
            .iter()
            .find(|volume| &volume.source == source)
        else {
            return menu;
        };
        let busy = storage.operation.is_some();
        let operating_on_volume = storage
            .operation
            .as_ref()
            .is_some_and(|operation| &operation.source == source);
        let mut menu = menu;
        for (label, target) in [
            ("在新标签页中打开", OpenVolumeTarget::NewTab),
            ("在新窗口中打开", OpenVolumeTarget::NewWindow),
        ] {
            let source = source.clone();
            menu = menu.item(
                app_menu_item(owner, label, None, move |app, window, cx| {
                    let volume = app
                        .storage
                        .read(cx)
                        .volumes
                        .iter()
                        .find(|volume| volume.source == source)
                        .cloned();
                    if let Some(volume) = volume {
                        app.open_volume_at(&volume, target, window, cx);
                    }
                })
                .disabled(operating_on_volume || (busy && volume.mountpoint.is_none())),
            );
        }
        let unmount = source.clone();
        let properties = source.clone();
        menu.separator()
            .item(
                app_menu_item(owner, "卸载", Some(IconName::Eject), move |app, _, cx| {
                    app.unmount_volume(&unmount, cx);
                })
                .disabled(busy || !volume.mounted),
            )
            .item(app_menu_item(
                owner,
                "属性",
                Some(IconName::Info),
                move |app, window, cx| {
                    let volume = app
                        .storage
                        .read(cx)
                        .volumes
                        .iter()
                        .find(|volume| volume.source == properties)
                        .cloned();
                    if let Some(volume) = volume {
                        app.open_volume_properties(volume, window, cx);
                    }
                },
            ))
    }

    fn open_volume_at(
        &mut self,
        volume: &Volume,
        target: OpenVolumeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pane_changes_allowed()
            || self
                .storage
                .read(cx)
                .operation
                .as_ref()
                .is_some_and(|operation| operation.source == volume.source)
        {
            return;
        }
        if let Some(path) = &volume.mountpoint {
            self.open_volume_path(path.clone(), target, window, cx);
            return;
        }
        if self.storage.read(cx).operation.is_some() {
            return;
        }
        if volume.mounted {
            self.storage.update(cx, |state, cx| {
                state.error = Some("手机已挂载，但没有可浏览的本地路径，请检查 gvfs-fuse".into());
                cx.notify();
            });
            return;
        }
        if !volume.locked {
            self.mount_volume(volume, target, None, window, cx);
            return;
        }

        let input = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("密码"));
        input.update(cx, |input, cx| input.focus(window, cx));
        self.volume_unlock = Some(VolumeUnlock {
            volume: volume.clone(),
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
        let volume = unlock.volume.clone();
        let target = unlock.target;
        unlock
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        unlock.error = None;
        unlock.running = true;
        self.mount_volume(&volume, target, Some(password), window, cx);
        cx.notify();
    }

    fn close_volume_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.volume_unlock.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    pub(super) fn volume_unlock_dialog(&self, cx: &mut Context<Self>) -> Option<Dialog> {
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
                                        .child(unlock.volume.label.clone()),
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

    fn mount_volume(
        &mut self,
        volume: &Volume,
        target: OpenVolumeTarget,
        password: Option<Zeroizing<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = self.storage.clone();
        let source = volume.source.clone();
        storage.update(cx, |state, cx| {
            state.operation = Some(VolumeOperation {
                source: source.clone(),
                unmounting: false,
            });
            state.error = None;
            cx.notify();
        });
        let pane_id = self.pane().id;
        let revision = self.pane().navigation_revision;
        let handle = window.window_handle().downcast::<Self>();
        let unlocking = password.is_some();
        let mount_source = source.clone();
        let work = cx.background_spawn(async move {
            let result = match password {
                Some(password) => mount::unlock_and_mount(&mount_source, password.as_str()),
                None => mount::mount(&mount_source).map_err(UnlockMountError::Mount),
            };
            let volumes = (!matches!(&result, Err(UnlockMountError::Unlock(_))))
                .then(|| mount::volumes().ok())
                .flatten();
            (result, volumes)
        });
        cx.spawn(async move |_, cx| {
            let (result, volumes) = work.await;
            cx.update(|cx| {
                storage.update(cx, |state, cx| {
                    state.finish_operation(cx);
                    if let Some(volumes) = volumes {
                        state.volumes = volumes;
                        state.discovery_error = None;
                    } else if let Ok(path) = &result
                        && let Some(volume) = state
                            .volumes
                            .iter_mut()
                            .find(|volume| volume.source == source)
                    {
                        volume.mountpoint = Some(path.clone());
                        volume.mounted = true;
                        volume.locked = false;
                    }
                    if let Err(UnlockMountError::Mount(error)) = &result
                        && !mount::is_authentication_dismissed(error)
                    {
                        state.error = Some(format!("无法挂载：{error}"));
                    }
                    cx.notify();
                });
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |this, window, cx| {
                        if unlocking {
                            if let Err(UnlockMountError::Unlock(error)) = &result
                                && !mount::is_authentication_dismissed(error)
                            {
                                if let Some(unlock) = &mut this.volume_unlock {
                                    unlock.running = false;
                                    unlock.error = Some(format!("无法解锁：{error}"));
                                    unlock.input.update(cx, |input, cx| input.focus(window, cx));
                                    cx.notify();
                                }
                                return;
                            }
                            this.close_volume_unlock(window, cx);
                        }
                        if let Ok(path) = result {
                            this.with_pane(pane_id, |this| {
                                if this.pane().navigation_revision == revision {
                                    this.open_volume_path(path, target, window, cx);
                                }
                            });
                        }
                    });
                }
            });
        })
        .detach();
    }
    fn open_volume_path(
        &mut self,
        path: PathBuf,
        target: OpenVolumeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            OpenVolumeTarget::CurrentTab => self.navigate(path, window, cx),
            OpenVolumeTarget::NewTab => self.new_tab(Some(Location::directory(path)), window, cx),
            OpenVolumeTarget::NewWindow => {
                if let Err(error) = Self::open_window_at(cx, Some((path, self.bookmarks.clone()))) {
                    self.workspace_error = Some(format!("无法打开新窗口：{error}"));
                    cx.notify();
                }
            }
        }
    }

    fn unmount_volume(&self, source: &VolumeSource, cx: &mut Context<Self>) {
        let storage = self.storage.clone();
        let state = storage.read(cx);
        if state.operation.is_some() {
            return;
        }
        let Some(volume) = state
            .volumes
            .iter()
            .find(|volume| &volume.source == source && volume.mounted)
        else {
            return;
        };
        let mountpoint = volume.mountpoint.clone();
        let source = source.clone();
        storage.update(cx, |state, cx| {
            state.operation = Some(VolumeOperation {
                source: source.clone(),
                unmounting: true,
            });
            state.error = None;
            cx.notify();
        });
        let work = cx.background_spawn(async move {
            let result = mount::unmount(&source);
            let volumes = if result.is_ok() {
                mount::volumes().ok()
            } else {
                None
            };
            (source, result, volumes)
        });
        cx.spawn(async move |_, cx| {
            let (source, result, volumes) = work.await;
            cx.update(|cx| {
                storage.update(cx, |state, cx| {
                    state.finish_operation(cx);
                    if let Some(volumes) = volumes {
                        state.volumes = volumes;
                        state.discovery_error = None;
                    } else if result.is_ok()
                        && let Some(volume) = state
                            .volumes
                            .iter_mut()
                            .find(|volume| volume.source == source)
                    {
                        volume.mounted = false;
                        volume.mountpoint = None;
                        volume.usage = None;
                    }
                    match result {
                        Err(error) => state.error = Some(format!("无法卸载：{error}")),
                        Ok(()) => {
                            if let Some(path) = mountpoint {
                                cx.emit(VolumeUnmounted(path));
                            }
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }
    pub(super) fn leave_unmounted_volume(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let destination = self
            .home()
            .filter(|home| !home.starts_with(path))
            .unwrap_or_else(|| Path::new("/"))
            .to_owned();
        let panes: Vec<_> = self.panes.iter().map(|pane| pane.id).collect();
        for id in panes {
            self.with_pane(id, |app| {
                if app
                    .location()
                    .is_some_and(|current| current.starts_with(path))
                {
                    app.navigate(destination.clone(), window, cx);
                }
            });
        }
    }
}
