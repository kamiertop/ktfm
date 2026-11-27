use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::prelude::*;
use gpui_kit::*;
use settings::config::tab::Location;
use sys::mount::{self, UnlockMountError, VolumeSource};
use zeroize::Zeroizing;

use super::App;
use super::menu::app_menu_item;

mod dialog;
mod storage;

pub(super) use dialog::{VolumeUnlock, volume_details};
pub(super) use storage::{SharedStorage, StorageDevices, VolumeOperation, VolumeUnmounted};

use storage::MountChange;

#[derive(Clone, Copy)]
enum OpenVolumeTarget {
    CurrentTab,
    NewTab,
    NewWindow,
}

impl App {
    pub(super) fn open_volume(
        &mut self,
        source: &VolumeSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_volume_at(source, OpenVolumeTarget::CurrentTab, window, cx);
    }

    pub(super) fn build_volume_menu(
        &self,
        menu: PopupMenu,
        source: &VolumeSource,
        owner: &WeakEntity<Self>,
        cx: &gpui_kit::App,
    ) -> PopupMenu {
        let storage = self.storage.read(cx);
        let Some(volume) = storage.volume(source) else {
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
                    app.open_volume_at(&source, target, window, cx);
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
                    if let Some(volume) = app.storage.read(cx).volume(&properties).cloned() {
                        app.open_volume_properties(volume, window, cx);
                    }
                },
            ))
    }

    fn open_volume_at(
        &mut self,
        source: &VolumeSource,
        target: OpenVolumeTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = self.storage.read(cx);
        if !self.pane_changes_allowed()
            || storage
                .operation
                .as_ref()
                .is_some_and(|operation| &operation.source == source)
        {
            return;
        }
        let Some(volume) = storage.volume(source) else {
            return;
        };
        if let Some(path) = &volume.mountpoint {
            self.open_volume_path(path.clone(), target, window, cx);
            return;
        }
        if storage.operation.is_some() {
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
            self.mount_volume(source.clone(), target, None, window, cx);
            return;
        }

        self.show_volume_unlock(volume.clone(), target, window, cx);
    }

    fn mount_volume(
        &mut self,
        source: VolumeSource,
        target: OpenVolumeTarget,
        password: Option<Zeroizing<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = self.storage.clone();
        storage.update(cx, |state, cx| {
            state.begin_operation(source.clone(), false, cx)
        });
        let pane_id = self.pane().id;
        let revision = self.pane().navigation_revision;
        let handle = window.window_handle().downcast::<Self>();
        let unlocking = password.is_some();
        let work = cx.background_spawn(async move {
            match password {
                Some(password) => mount::unlock_and_mount(&source, password.as_str()),
                None => mount::mount(&source).map_err(UnlockMountError::Mount),
            }
        });
        cx.spawn(async move |_, cx| {
            let result = work.await;
            cx.update(|cx| {
                storage.update(cx, |state, cx| {
                    let change = match &result {
                        Ok(path) => MountChange::Mounted(path.clone()),
                        Err(_) => MountChange::Unchanged,
                    };
                    let error = match &result {
                        Err(UnlockMountError::Mount(error))
                            if !mount::is_authentication_dismissed(error) =>
                        {
                            Some(format!("无法挂载：{error}"))
                        }
                        _ => None,
                    };
                    state.finish_operation(change, error, cx);
                });
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |this, window, cx| {
                        if unlocking {
                            if let Err(UnlockMountError::Unlock(error)) = &result
                                && !mount::is_authentication_dismissed(error)
                            {
                                this.show_volume_unlock_error(error, window, cx);
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
        if !state.volume(source).is_some_and(|volume| volume.mounted) {
            return;
        }
        let source = source.clone();
        storage.update(cx, |state, cx| {
            state.begin_operation(source.clone(), true, cx)
        });
        let work = cx.background_spawn(async move { mount::unmount(&source) });
        cx.spawn(async move |_, cx| {
            let result = work.await;
            cx.update(|cx| {
                storage.update(cx, |state, cx| {
                    let (change, error) = match result {
                        Ok(()) => (MountChange::Unmounted, None),
                        Err(error) if mount::is_authentication_dismissed(&error) => {
                            (MountChange::Unchanged, None)
                        }
                        Err(error) => (MountChange::Unchanged, Some(format!("无法卸载：{error}"))),
                    };
                    state.finish_operation(change, error, cx);
                });
            });
        })
        .detach();
    }
}
