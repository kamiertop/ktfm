#[cfg(not(target_os = "linux"))]
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub device: PathBuf,
    pub label: String,
    pub mountpoint: Option<PathBuf>,
    pub removable: bool,
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde::Deserialize;

    use super::Volume;

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
        #[serde(rename = "fstype")]
        fs_type: Option<String>,
        #[serde(rename = "mountpoints")]
        mount_points: Option<Vec<Option<PathBuf>>>,
        #[serde(default)]
        children: Vec<BlockDevice>,
    }

    pub fn volumes() -> io::Result<Vec<Volume>> {
        let output = Command::new("lsblk")
            .args([
                "--json",
                "--paths",
                "--output",
                "NAME,TYPE,RM,TRAN,LABEL,FSTYPE,MOUNTPOINTS",
            ])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        parse_volumes(&output.stdout)
    }

    fn parse_volumes(json: &[u8]) -> io::Result<Vec<Volume>> {
        let devices: BlockDevices = serde_json::from_slice(json)?;
        let mut volumes = Vec::new();
        for device in devices.block_devices {
            collect(device, false, &mut volumes);
        }
        volumes.sort_unstable_by(|a, b| (&a.label, &a.device).cmp(&(&b.label, &b.device)));
        Ok(volumes)
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
        let mountable = matches!(device.kind.as_str(), "disk" | "part")
            && device.fs_type.as_deref().is_some_and(|kind| {
                !matches!(
                    kind,
                    "swap" | "crypto_LUKS" | "LVM2_member" | "linux_raid_member"
                )
            });
        let unmounted = mountpoints.iter().all(Option::is_none);
        if (mountpoint.is_some() || (unmounted && removable && mountable))
            && !volumes.iter().any(|volume| volume.device == device.name)
        {
            let label = device.label.unwrap_or_else(|| {
                let name = mountpoint.as_ref().unwrap_or(&device.name);
                name.file_name()
                    .unwrap_or(name.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            });
            volumes.push(Volume {
                device: device.name,
                label,
                mountpoint,
                removable,
            });
        }
        for child in device.children {
            collect(child, removable, volumes);
        }
    }

    pub fn mount(device: &Path) -> io::Result<PathBuf> {
        // 参数直接交给 UDisks，不经过 shell。
        let output = Command::new("udisksctl")
            .args(["mount", "--block-device"])
            .arg(device)
            .output()?;
        let result = volumes()?
            .into_iter()
            .find(|volume| volume.device == device)
            .and_then(|volume| volume.mountpoint);
        result.ok_or_else(|| {
            let message = String::from_utf8_lossy(&output.stderr);
            if output.status.success() {
                io::Error::other("已挂载，但未找到可浏览的挂载点")
            } else {
                io::Error::other(message.trim().to_owned())
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn includes_usb_partitions_and_data_mounts_only() {
            let volumes = parse_volumes(br#"{"blockdevices":[
                {"name":"/dev/sda","type":"disk","rm":false,"tran":"usb","children":[
                    {"name":"/dev/sda1","type":"part","rm":false,"label":"USB","fstype":"exfat","mountpoints":[null]},
                    {"name":"/dev/sda2","type":"part","rm":false,"fstype":"swap","mountpoints":[]}
                ]},
                {"name":"/dev/sdb1","type":"part","rm":false,"fstype":"ext4","mountpoints":["/mnt/data"]},
                {"name":"/dev/sdb2","type":"part","rm":false,"fstype":"ext4","mountpoints":["/"]}
            ]}"#).unwrap();
            assert_eq!(volumes.len(), 2);
            assert!(volumes[0].removable);
            assert_eq!(volumes[1].mountpoint, Some("/mnt/data".into()));
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{mount, volumes};

#[cfg(not(target_os = "linux"))]
pub fn volumes() -> io::Result<Vec<Volume>> {
    Ok(Vec::new())
}

#[cfg(not(target_os = "linux"))]
pub fn mount(_: &std::path::Path) -> io::Result<PathBuf> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}
