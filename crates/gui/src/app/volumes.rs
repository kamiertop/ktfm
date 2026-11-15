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
}

impl App {
    pub(super) fn watch_volumes(&self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            loop {
                let volumes = executor.spawn(async { mount::volumes() }).await;
                if this
                    .update(cx, |this, cx| {
                        let error = match volumes {
                            Ok(volumes) => {
                                if this.storage.volumes != volumes {
                                    this.storage.volumes = volumes;
                                    cx.notify();
                                }
                                None
                            }
                            Err(error) => Some(format!("无法读取设备：{error}")),
                        };
                        if this.storage.discovery_error != error {
                            this.storage.discovery_error = error;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                executor.timer(Duration::from_secs(3)).await;
            }
        })
        .detach();
    }

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
        if self.storage.mounting.is_some() {
            return;
        }
        let device = volume.device.clone();
        self.storage.mounting = Some(device.clone());
        self.storage.error = None;
        let generation = self.browser.generation;
        let mount_device = device.clone();
        let mount = cx.background_spawn(async move { mount::mount(&mount_device) });
        cx.spawn_in(window, async move |this, cx| {
            let result = mount.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.storage.mounting = None;
                match result {
                    Ok(path) => {
                        if let Some(volume) = this
                            .storage
                            .volumes
                            .iter_mut()
                            .find(|volume| volume.device == device)
                        {
                            volume.mountpoint = Some(path.clone());
                        }
                        if this.browser.generation == generation {
                            this.navigate(path, window, cx);
                        }
                    }
                    Err(error) => this.storage.error = Some(format!("无法挂载：{error}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
