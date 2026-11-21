use std::io;
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use sys::mount::{self, Volume, VolumeSource};

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(in crate::app) struct StorageDevices {
    pub(in crate::app) volumes: Vec<Volume>,
    pub(in crate::app) operation: Option<VolumeOperation>,
    pub(in crate::app) error: Option<String>,
    discovery_errors: [Option<String>; 2],
    operation_revision: u64,
    volumes_revision: u64,
}

pub(in crate::app) struct VolumeUnmounted(pub(in crate::app) PathBuf);

impl EventEmitter<VolumeUnmounted> for StorageDevices {}

pub(in crate::app) struct VolumeOperation {
    pub(in crate::app) source: VolumeSource,
    pub(in crate::app) unmounting: bool,
}

#[derive(Clone, Copy)]
enum DeviceKind {
    Block = 0,
    Phone = 1,
}

impl DeviceKind {
    fn matches(self, source: &VolumeSource) -> bool {
        matches!(
            (self, source),
            (Self::Block, VolumeSource::Block(_)) | (Self::Phone, VolumeSource::Phone(_))
        )
    }

    fn scan(self) -> io::Result<Vec<Volume>> {
        match self {
            Self::Block => mount::block_volumes(),
            Self::Phone => mount::phone_volumes(),
        }
    }
}

pub(super) enum MountChange {
    Mounted(PathBuf),
    Unmounted,
    Unchanged,
}

impl StorageDevices {
    pub(in crate::app) fn volume(&self, source: &VolumeSource) -> Option<&Volume> {
        self.volumes.iter().find(|volume| &volume.source == source)
    }

    pub(in crate::app) fn discovery_error(&self) -> Option<&String> {
        self.discovery_errors.iter().flatten().next()
    }

    pub(super) fn begin_operation(
        &mut self,
        source: VolumeSource,
        unmounting: bool,
        cx: &mut Context<Self>,
    ) {
        self.operation = Some(VolumeOperation { source, unmounting });
        self.error = None;
        cx.notify();
    }

    pub(super) fn finish_operation(
        &mut self,
        change: MountChange,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let operation = self.operation.take().expect("设备操作已开始");
        self.operation_revision += 1;
        self.error = error;
        let mountpoint = match change {
            MountChange::Mounted(path) => Some(Some(path)),
            MountChange::Unmounted => Some(None),
            MountChange::Unchanged => None,
        };
        if let Some(mountpoint) = mountpoint {
            let mut volumes = self.volumes.clone();
            if let Some(volume) = volumes
                .iter_mut()
                .find(|volume| volume.source == operation.source)
            {
                volume.mounted = mountpoint.is_some();
                volume.mountpoint = mountpoint;
                volume.locked = false;
                volume.usage = None;
            }
            self.replace_volumes(volumes, cx);
        }
        cx.global::<SharedStorage>().request_discovery();
        cx.notify();
    }

    fn apply_scan(
        &mut self,
        kind: DeviceKind,
        result: io::Result<Vec<Volume>>,
        cx: &mut Context<Self>,
    ) {
        let error = match result {
            Ok(mut volumes) => {
                // 等操作返回后再更新该设备，其他设备仍能响应插拔。
                if let Some(operation) = &self.operation
                    && kind.matches(&operation.source)
                    && let Some(current) = self.volume(&operation.source)
                {
                    volumes.retain(|volume| volume.source != operation.source);
                    volumes.push(current.clone());
                }
                for volume in &mut volumes {
                    if let Some(old) = self.volume(&volume.source)
                        && old.mountpoint == volume.mountpoint
                    {
                        volume.usage = old.usage;
                    }
                }
                volumes.extend(
                    self.volumes
                        .iter()
                        .filter(|volume| !kind.matches(&volume.source))
                        .cloned(),
                );
                self.replace_volumes(volumes, cx);
                None
            }
            Err(error) => Some(format!("无法读取设备：{error}")),
        };
        let current = &mut self.discovery_errors[kind as usize];
        if *current != error {
            *current = error;
            cx.notify();
        }
    }

    fn replace_volumes(&mut self, mut volumes: Vec<Volume>, cx: &mut Context<Self>) {
        volumes.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.source.cmp(&right.source))
        });
        if self.volumes == volumes {
            return;
        }
        let removed = removed_mountpoints(&self.volumes, &volumes);
        self.volumes = volumes;
        self.volumes_revision += 1;
        for path in removed {
            cx.emit(VolumeUnmounted(path));
        }
        SharedStorage::request_usage_refresh(cx);
        cx.notify();
    }
}

fn removed_mountpoints(previous: &[Volume], current: &[Volume]) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for path in previous
        .iter()
        .filter_map(|volume| volume.mountpoint.as_deref())
    {
        if !current
            .iter()
            .any(|volume| volume.mountpoint.as_deref() == Some(path))
            && !removed
                .iter()
                .any(|previous: &PathBuf| previous.as_path() == path)
        {
            removed.push(path.to_owned());
        }
    }
    removed
}

pub(in crate::app) struct SharedStorage {
    pub(in crate::app) state: Entity<StorageDevices>,
    discovery: [async_channel::Sender<()>; 2],
    usage: async_channel::Sender<()>,
}

impl Global for SharedStorage {}

impl SharedStorage {
    pub(in crate::app) fn install(cx: &mut App) {
        let state = cx.new(|_| StorageDevices::default());
        let (blocks, block_requests) = async_channel::bounded(1);
        let (phones, phone_requests) = async_channel::bounded(1);
        let (usage, usage_requests) = async_channel::bounded(1);
        let discovery = [blocks, phones];
        cx.set_global(Self {
            state: state.clone(),
            discovery: discovery.clone(),
            usage,
        });

        Self::scan_devices(state.clone(), DeviceKind::Block, block_requests, cx);
        Self::scan_devices(state.clone(), DeviceKind::Phone, phone_requests, cx);
        Self::refresh_usage(state, usage_requests, cx);
        #[cfg(target_os = "linux")]
        Self::watch_devices(discovery, cx);
    }

    fn scan_devices(
        storage: Entity<StorageDevices>,
        kind: DeviceKind,
        requests: async_channel::Receiver<()>,
        cx: &mut App,
    ) {
        let mut revision = storage.read(cx).operation_revision;
        let mut scan = cx.background_spawn(async move { kind.scan() });
        cx.spawn(async move |cx| {
            loop {
                let result = scan.await;
                let failed = result.is_err();
                cx.update(|cx| {
                    storage.update(cx, |state, cx| {
                        if state.operation_revision == revision {
                            state.apply_scan(kind, result, cx);
                        }
                    })
                });
                let requested = if failed {
                    let retry = cx.background_executor().timer(REFRESH_INTERVAL);
                    futures_lite::future::or(requests.recv(), async {
                        retry.await;
                        Ok(())
                    })
                    .await
                } else {
                    requests.recv().await
                };
                if requested.is_err() {
                    break;
                }
                // 一次插拔可能产生多条卷和挂载事件。
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let _ = requests.try_recv();
                revision = cx.update(|cx| storage.read(cx).operation_revision);
                scan = cx.background_spawn(async move { kind.scan() });
            }
        })
        .detach();
    }

    fn refresh_usage(
        storage: Entity<StorageDevices>,
        requests: async_channel::Receiver<()>,
        cx: &mut App,
    ) {
        cx.spawn(async move |cx| {
            loop {
                let timer = cx.background_executor().timer(REFRESH_INTERVAL);
                if futures_lite::future::or(requests.recv(), async {
                    timer.await;
                    Ok(())
                })
                .await
                .is_err()
                {
                    break;
                }
                let Some((revision, mut volumes)) = cx.update(|cx| {
                    let state = storage.read(cx);
                    (state.operation.is_none()
                        && state
                            .volumes
                            .iter()
                            .any(|volume| volume.mountpoint.is_some()))
                    .then(|| (state.volumes_revision, state.volumes.clone()))
                }) else {
                    continue;
                };
                let refreshed = cx
                    .background_spawn(async move {
                        mount::refresh_usage(&mut volumes);
                        volumes
                    })
                    .await;
                cx.update(|cx| {
                    storage.update(cx, |state, cx| {
                        if state.volumes_revision != revision || state.operation.is_some() {
                            return;
                        }
                        let mut changed = false;
                        for (volume, refreshed) in state.volumes.iter_mut().zip(refreshed) {
                            changed |= volume.usage != refreshed.usage;
                            volume.usage = refreshed.usage;
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                });
            }
        })
        .detach();
    }

    #[cfg(target_os = "linux")]
    fn watch_devices(discovery: [async_channel::Sender<()>; 2], cx: &mut App) {
        let executor = cx.background_executor().clone();
        let (stop, stopped) = async_channel::bounded::<()>(1);
        let mut watcher = Some(cx.background_spawn(async move {
            let refresh = || request_discovery(&discovery);
            futures_lite::future::or(
                async {
                    let _ = stopped.recv().await;
                },
                async {
                    loop {
                        if let Err(error) = mount::watch_volumes(refresh).await {
                            tracing::warn!(%error, "设备监听不可用，暂用低频扫描");
                        }
                        refresh();
                        executor.timer(REFRESH_INTERVAL).await;
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

    fn request_discovery(&self) {
        request_discovery(&self.discovery);
    }

    pub(in crate::app) fn request_usage_refresh(cx: &App) {
        let _ = cx.global::<Self>().usage.try_send(());
    }
}

fn request_discovery(requests: &[async_channel::Sender<()>; 2]) {
    for request in requests {
        let _ = request.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::{PathBuf, Volume, VolumeSource, removed_mountpoints};

    fn mounted(source: &str, mountpoint: &str) -> Volume {
        Volume {
            source: VolumeSource::Block(source.into()),
            label: source.into(),
            mounted: true,
            mountpoint: Some(mountpoint.into()),
            removable: false,
            locked: false,
            capacity: None,
            usage: None,
        }
    }

    #[test]
    fn only_missing_mountpoints_are_reported_once() {
        let old = [
            mounted("/dev/sda", "/mnt/keep"),
            mounted("/dev/sdb", "/mnt/remove"),
            mounted("/dev/sdc", "/mnt/remove"),
        ];
        let new = [mounted("/dev/mapper/keep", "/mnt/keep")];
        assert_eq!(
            removed_mountpoints(&old, &new),
            [PathBuf::from("/mnt/remove")]
        );
    }
}
