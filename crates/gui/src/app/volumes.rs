use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use sys::mount::{self, Volume};

use super::App;

#[derive(Default)]
pub(super) struct StorageDevices {
    pub(super) volumes: Vec<Volume>,
    pub(super) mounting: Option<PathBuf>,
    pub(super) error: Option<String>,
    pub(super) discovery_error: Option<String>,
    revision: u64,
}

pub(super) struct SharedStorage(pub Entity<StorageDevices>);

impl Global for SharedStorage {}

impl SharedStorage {
    pub(super) fn install(cx: &mut gpui_kit::App) {
        let state = cx.new(|_| StorageDevices::default());
        cx.set_global(Self(state.clone()));
        cx.spawn(async move |cx| {
            loop {
                let revision = cx.update(|cx| state.read(cx).revision);
                let volumes = cx.background_spawn(async { mount::volumes() }).await;
                cx.update(|cx| {
                    state.update(cx, |state, cx| {
                        if revision != state.revision || state.mounting.is_some() {
                            return;
                        }
                        let error = match volumes {
                            Ok(volumes) => {
                                if state.volumes != volumes {
                                    state.volumes = volumes;
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
                    })
                });
                cx.background_executor().timer(Duration::from_secs(3)).await;
            }
        })
        .detach();
    }
}

impl App {
    pub(super) fn open_volume(
        &mut self,
        volume: &Volume,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = &volume.mountpoint {
            self.navigate(path.clone(), window, cx);
            return;
        }
        let storage = self.storage.clone();
        if storage.read(cx).mounting.is_some() {
            return;
        }
        let device = volume.device.clone();
        storage.update(cx, |state, cx| {
            state.mounting = Some(device.clone());
            state.error = None;
            cx.notify();
        });
        let pane_id = self.pane().id;
        let revision = self.pane().navigation_revision;
        let handle = window.window_handle().downcast::<Self>();
        let mount_device = device.clone();
        let mount = cx.background_spawn(async move { mount::mount(&mount_device) });
        cx.spawn(async move |_, cx| {
            let result = mount.await;
            cx.update(|cx| {
                storage.update(cx, |state, cx| {
                    state.mounting = None;
                    state.revision += 1;
                    match &result {
                        Ok(path) => {
                            if let Some(volume) = state
                                .volumes
                                .iter_mut()
                                .find(|volume| volume.device == device)
                            {
                                volume.mountpoint = Some(path.clone());
                            }
                        }
                        Err(error) => state.error = Some(format!("无法挂载：{error}")),
                    }
                    cx.notify();
                });
                if let (Some(handle), Ok(path)) = (handle, result) {
                    let _ = handle.update(cx, |this, window, cx| {
                        this.with_pane(pane_id, |this| {
                            if this.pane().navigation_revision == revision {
                                this.navigate(path, window, cx);
                            }
                        });
                    });
                }
            });
        })
        .detach();
    }
}
