use std::io;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VolumeSource {
    Block(PathBuf),
    Phone(String),
}

impl std::fmt::Display for VolumeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block(path) => write!(f, "{}", path.display()),
            Self::Phone(uri) => f.write_str(uri),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub source: VolumeSource,
    pub mounted: bool,
    pub label: String,
    pub mountpoint: Option<PathBuf>,
    pub removable: bool,
    pub locked: bool,
    pub capacity: Option<u64>,
    pub usage: Option<VolumeUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeUsage {
    pub total: u64,
    pub used: u64,
}

pub enum UnlockMountError {
    Unlock(io::Error),
    Mount(io::Error),
}

#[cfg(target_os = "linux")]
mod phones;
#[cfg(target_os = "linux")]
mod watch;

#[cfg(target_os = "linux")]
pub use watch::watch_volumes;

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::os::unix::ffi::OsStringExt as _;
    use std::process::Command;

    use serde::Deserialize;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    use super::{Path, PathBuf, UnlockMountError, Volume, VolumeSource, VolumeUsage, io};

    use super::phones;

    const UDISKS: &str = "org.freedesktop.UDisks2";

    pub fn is_authentication_dismissed(error: &io::Error) -> bool {
        error
            .get_ref()
            .and_then(|error| error.downcast_ref())
            .is_some_and(authentication_dismissed)
    }

    fn authentication_dismissed(error: &zbus::Error) -> bool {
        matches!(error, zbus::Error::MethodError(name, _, _)
            if name.as_str() == "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed")
    }

    #[derive(Deserialize)]
    struct BlockDevices {
        #[serde(rename = "blockdevices")]
        block_devices: Vec<BlockDevice>,
    }

    #[derive(Deserialize)]
    struct BlockDevice {
        name: PathBuf,
        #[serde(rename = "type")]
        kind: String,
        rm: bool,
        tran: Option<String>,
        label: Option<String>,
        size: Option<u64>,
        parttype: Option<String>,
        #[serde(rename = "fstype")]
        fs_type: Option<String>,
        #[serde(rename = "mountpoints")]
        mount_points: Option<Vec<Option<PathBuf>>>,
        #[serde(default)]
        children: Vec<BlockDevice>,
    }

    pub fn block_volumes() -> io::Result<Vec<Volume>> {
        let output = Command::new("lsblk")
            .args([
                "--json",
                "--paths",
                "--bytes",
                "--output",
                "NAME,TYPE,RM,TRAN,LABEL,FSTYPE,MOUNTPOINTS,SIZE,PARTTYPE",
            ])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        parse_volumes(&output.stdout)
    }

    pub fn phone_volumes() -> io::Result<Vec<Volume>> {
        phones::volumes()
    }

    pub fn refresh_usage(volumes: &mut [Volume]) {
        for volume in volumes {
            volume.usage = volume.mountpoint.as_deref().and_then(filesystem_usage);
        }
    }

    fn filesystem_usage(path: &Path) -> Option<VolumeUsage> {
        let stats = rustix::fs::statvfs(path).ok()?;
        let total = stats.f_blocks.checked_mul(stats.f_frsize)?;
        (total > 0).then_some(VolumeUsage {
            total,
            used: stats.f_blocks.saturating_sub(stats.f_bfree) * stats.f_frsize,
        })
    }

    fn parse_volumes(json: &[u8]) -> io::Result<Vec<Volume>> {
        let devices: BlockDevices = serde_json::from_slice(json)?;
        let mut volumes = Vec::new();
        for device in devices.block_devices {
            collect(device, false, &mut volumes);
        }
        volumes.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(volumes)
    }

    fn is_system_partition(parttype: &str) -> bool {
        matches!(
            parttype,
            "0xef" | "0x27"
                | "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" // EFI
                | "21686148-6449-6e6f-744e-656564454649" // BIOS 引导
                | "e3c9e316-0b5c-4db8-817d-f92df00215ae" // Windows 保留
                | "de94bba4-06d1-4d40-a16a-bfd50179d6ac" // Windows 恢复
        )
    }

    fn collect(device: BlockDevice, inherited_removable: bool, volumes: &mut Vec<Volume>) {
        let removable = inherited_removable || device.rm || device.tran.as_deref() == Some("usb");
        let mountpoints = device.mount_points.unwrap_or_default();
        let mountpoint = mountpoints
            .iter()
            .flatten()
            .find(|path| {
                ["/run/media", "/media", "/mnt"]
                    .iter()
                    .any(|root| path.starts_with(root))
            })
            .cloned();
        let fs_type = device.fs_type.as_deref().unwrap_or_default();
        let encrypted = fs_type == "crypto_LUKS" || fs_type.eq_ignore_ascii_case("BitLocker");
        let locked = encrypted && !device.children.iter().any(|child| child.kind == "crypt");
        let mountable = !encrypted
            && !fs_type.is_empty()
            && !matches!(
                fs_type,
                "swap" | "LVM2_member" | "linux_raid_member" | "zfs_member" | "crypto_unknown"
            );
        let unmounted = mountpoints.iter().all(Option::is_none);
        let data_device = matches!(device.kind.as_str(), "disk" | "part" | "crypt" | "rom")
            && !device.parttype.as_deref().is_some_and(is_system_partition);
        if (mountpoint.is_some() || (unmounted && data_device && (locked || mountable)))
            && !volumes.iter().any(|volume| matches!(&volume.source, VolumeSource::Block(path) if path == &device.name))
        {
            let label = device
                .label
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| {
                    let name = mountpoint.as_ref().unwrap_or(&device.name);
                    name.file_name()
                        .unwrap_or(name.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                });
            volumes.push(Volume {
                source: VolumeSource::Block(device.name),
                mounted: mountpoint.is_some(),
                label,
                mountpoint,
                removable,
                locked,
                capacity: device.size.filter(|size| *size > 0),
                usage: None,
            });
        }
        for child in device.children {
            collect(child, removable, volumes);
        }
    }

    fn resolve_device(connection: &Connection, device: &Path) -> io::Result<OwnedObjectPath> {
        let path = device.to_str().ok_or(io::ErrorKind::InvalidInput)?;
        let manager = Proxy::new(
            connection,
            UDISKS,
            "/org/freedesktop/UDisks2/Manager",
            "org.freedesktop.UDisks2.Manager",
        )
        .map_err(io::Error::other)?;
        let options = HashMap::<&str, Value<'_>>::new();
        let devices: Vec<OwnedObjectPath> = manager
            .call(
                "ResolveDevice",
                &(HashMap::from([("path", Value::from(path))]), options),
            )
            .map_err(io::Error::other)?;
        devices
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "找不到该设备"))
    }

    fn mount_object(connection: &Connection, object: OwnedObjectPath) -> io::Result<PathBuf> {
        let filesystem = Proxy::new(
            connection,
            UDISKS,
            object,
            "org.freedesktop.UDisks2.Filesystem",
        )
        .map_err(io::Error::other)?;
        let options = HashMap::<&str, Value<'_>>::new();
        match filesystem.call::<_, _, String>("Mount", &(options,)) {
            Ok(path) => Ok(path.into()),
            Err(error) if authentication_dismissed(&error) => Err(io::Error::other(error)),
            Err(error) => {
                // 设备可能刚被其他文件管理器挂载。
                let paths = filesystem
                    .get_property::<Vec<Vec<u8>>>("MountPoints")
                    .unwrap_or_default();
                paths
                    .into_iter()
                    .find_map(|mut path| {
                        if path.last() == Some(&0) {
                            path.pop();
                        }
                        (!path.is_empty())
                            .then(|| PathBuf::from(std::ffi::OsString::from_vec(path)))
                    })
                    .ok_or_else(|| io::Error::other(error))
            }
        }
    }

    pub fn mount(source: &VolumeSource) -> io::Result<PathBuf> {
        match source {
            VolumeSource::Block(device) => {
                let connection = Connection::system().map_err(io::Error::other)?;
                let object = resolve_device(&connection, device)?;
                mount_object(&connection, object)
            }
            VolumeSource::Phone(uri) => phones::mount(uri),
        }
    }

    pub fn unmount(source: &VolumeSource) -> io::Result<()> {
        match source {
            VolumeSource::Block(device) => {
                let connection = Connection::system().map_err(io::Error::other)?;
                let object = resolve_device(&connection, device)?;
                let filesystem = Proxy::new(
                    &connection,
                    UDISKS,
                    object,
                    "org.freedesktop.UDisks2.Filesystem",
                )
                .map_err(io::Error::other)?;
                let options = HashMap::<&str, Value<'_>>::new();
                filesystem
                    .call("Unmount", &(options,))
                    .map_err(io::Error::other)
            }
            VolumeSource::Phone(uri) => phones::unmount(uri),
        }
    }

    pub fn unlock_and_mount(
        source: &VolumeSource,
        passphrase: &str,
    ) -> Result<PathBuf, UnlockMountError> {
        let VolumeSource::Block(device) = source else {
            return Err(UnlockMountError::Unlock(io::ErrorKind::Unsupported.into()));
        };
        let connection = Connection::system()
            .map_err(io::Error::other)
            .map_err(UnlockMountError::Unlock)?;
        let object = resolve_device(&connection, device).map_err(UnlockMountError::Unlock)?;
        let encrypted = Proxy::new(
            &connection,
            UDISKS,
            object,
            "org.freedesktop.UDisks2.Encrypted",
        )
        .map_err(io::Error::other)
        .map_err(UnlockMountError::Unlock)?;
        let cleartext = encrypted
            .get_property::<OwnedObjectPath>("CleartextDevice")
            .ok()
            .filter(|path| path.as_str() != "/");
        let unlocked = if let Some(cleartext) = cleartext {
            cleartext
        } else {
            let options = HashMap::<&str, Value<'_>>::new();
            encrypted
                .call("Unlock", &(passphrase, options))
                .map_err(|error| {
                    if authentication_dismissed(&error) {
                        return UnlockMountError::Unlock(io::Error::other(error));
                    }
                    // 解锁失败时只显示错误类型，不把 D-Bus 服务返回的详细信息带到界面或日志。
                    let message = match &error {
                        zbus::Error::MethodError(name, _, _)
                            if name.as_str().contains("NotAuthorized") =>
                        {
                            "没有解锁该设备的权限"
                        }
                        _ => "请检查密码和设备连接后重试",
                    };
                    UnlockMountError::Unlock(io::Error::other(message))
                })?
        };
        mount_object(&connection, unlocked).map_err(UnlockMountError::Mount)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn distinguishes_dismissed_authentication_from_denied_authorization() {
            let call = zbus::Message::method_call("/", "Mount")
                .unwrap()
                .build(&())
                .unwrap();
            for (name, dismissed) in [
                ("org.freedesktop.UDisks2.Error.NotAuthorizedDismissed", true),
                ("org.freedesktop.UDisks2.Error.NotAuthorized", false),
            ] {
                let reply = zbus::Message::error(&call.header(), name)
                    .unwrap()
                    .build(&"认证未完成")
                    .unwrap();
                let error = io::Error::other(zbus::Error::from(reply));
                assert_eq!(is_authentication_dismissed(&error), dismissed);
            }
        }

        #[test]
        fn discovers_locked_fixed_usb_and_mounted_data_volumes() {
            let volumes = parse_volumes(br#"{"blockdevices":[
                {"name":"/dev/sda","type":"disk","rm":false,"tran":"usb","children":[
                    {"name":"/dev/sda1","type":"part","rm":false,"label":"USB","fstype":"exfat","mountpoints":[null],"size":100},
                    {"name":"/dev/sda2","type":"part","rm":false,"fstype":"swap","mountpoints":[]}
                ]},
                {"name":"/dev/sdb1","type":"part","rm":false,"fstype":"ext4","mountpoints":["/mnt/data"],"size":200},
                {"name":"/dev/sdb2","type":"part","rm":false,"fstype":"ext4","mountpoints":["/"]},
                {"name":"/dev/sdb3","type":"part","rm":false,"fstype":"BitLocker","size":300},
                {"name":"/dev/sdb4","type":"part","rm":false,"fstype":"ntfs","parttype":"de94bba4-06d1-4d40-a16a-bfd50179d6ac"},
                {"name":"/dev/sdb5","type":"part","rm":false,"fstype":"ntfs"},
                {"name":"/dev/sdc1","type":"part","rm":false,"fstype":"crypto_LUKS","children":[
                    {"name":"/dev/mapper/vault","type":"crypt","rm":false,"fstype":"ext4","size":400}
                ]}
            ]}"#).unwrap();
            assert_eq!(volumes.len(), 5);
            assert!(volumes[0].removable);
            assert_eq!(volumes[0].capacity, Some(100));
            assert_eq!(volumes[1].mountpoint, Some("/mnt/data".into()));
            assert!(volumes[2].locked);
            assert_eq!(volumes[2].capacity, Some(300));
            assert!(!volumes[3].locked);
            assert_eq!(
                volumes[4].source,
                VolumeSource::Block("/dev/mapper/vault".into())
            );
            assert!(!volumes[4].locked);
        }

        #[test]
        fn reads_mounted_filesystem_usage() {
            let usage = filesystem_usage(Path::new("/")).unwrap();
            assert!(usage.total > 0 && usage.used <= usage.total);
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{
    block_volumes, is_authentication_dismissed, mount, phone_volumes, refresh_usage,
    unlock_and_mount, unmount,
};

#[cfg(not(target_os = "linux"))]
pub fn refresh_usage(_: &mut [Volume]) {}

#[cfg(not(target_os = "linux"))]
pub fn is_authentication_dismissed(_: &io::Error) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn block_volumes() -> io::Result<Vec<Volume>> {
    Ok(Vec::new())
}

#[cfg(not(target_os = "linux"))]
pub fn phone_volumes() -> io::Result<Vec<Volume>> {
    Ok(Vec::new())
}

#[cfg(not(target_os = "linux"))]
pub fn mount(_: &VolumeSource) -> io::Result<PathBuf> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(not(target_os = "linux"))]
pub fn unlock_and_mount(_: &VolumeSource, _: &str) -> Result<PathBuf, UnlockMountError> {
    Err(UnlockMountError::Unlock(io::ErrorKind::Unsupported.into()))
}

#[cfg(not(target_os = "linux"))]
pub fn unmount(_: &VolumeSource) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
