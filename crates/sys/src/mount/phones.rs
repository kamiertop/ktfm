use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use super::{Volume, VolumeSource};

struct GioVolume {
    label: String,
    mounted: bool,
}

pub(super) fn volumes() -> io::Result<Vec<Volume>> {
    // 枚举在独立的 GIO 进程内执行，让 GVfs 的卷监视器在该进程主线程初始化。
    let mut mounted_paths = fuse_mounts();
    let list = match run(&["mount", "--list", "--detail"]) {
        Ok(list) => list,
        Err(error) if mounted_paths.is_empty() => return Err(error),
        Err(_) => String::new(),
    };
    let mut volumes: Vec<_> = parse_volumes(&list)
        .into_iter()
        .map(|(uri, GioVolume { label, mounted })| {
            let mountpoint = mounted_paths
                .remove(&phone_key(&uri))
                .map(|(_, path)| path)
                .or_else(|| mounted.then(|| local_path(&uri)).flatten());
            phone_volume(uri, label, mounted || mountpoint.is_some(), mountpoint)
        })
        .collect();
    // 会话总线暂时不可用时，仍可发现已经通过 GVfs FUSE 暴露的手机。
    volumes.extend(mounted_paths.into_values().map(|(uri, path)| {
        let host = phone_key(&uri).1;
        let label = host
            .rsplit_once('_')
            .filter(|(_, suffix)| {
                suffix.len() >= 8 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .map_or(host.as_str(), |(name, _)| name)
            .replace('_', " ");
        phone_volume(uri, label, true, Some(path))
    }));
    Ok(volumes)
}

fn phone_volume(uri: String, label: String, mounted: bool, mountpoint: Option<PathBuf>) -> Volume {
    Volume {
        mountpoint,
        source: VolumeSource::Phone(uri),
        label,
        mounted,
        removable: true,
        locked: false,
        capacity: None,
        usage: None,
    }
}

fn fuse_mounts() -> BTreeMap<(String, String), (String, PathBuf)> {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return BTreeMap::new();
    };
    let Ok(entries) = std::fs::read_dir(PathBuf::from(runtime).join("gvfs")) else {
        return BTreeMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let uri = fuse_phone_uri(&name)?;
            Some((phone_key(&uri), (uri, entry.path())))
        })
        .collect()
}

fn fuse_phone_uri(name: &str) -> Option<String> {
    let (scheme, host) = name.split_once(":host=")?;
    phone_uri(&format!("{scheme}://{host}"))
}

fn phone_key(uri: &str) -> (String, String) {
    let (scheme, rest) = uri.split_once("://").unwrap_or_default();
    let host = rest.split('/').next().unwrap_or_default();
    let escaped_plus = host.replace('+', "%2B");
    let host = url::form_urlencoded::parse(escaped_plus.as_bytes())
        .next()
        .map_or_else(String::new, |(name, _)| name.into_owned());
    (scheme.to_owned(), host)
}

fn parse_volumes(list: &str) -> BTreeMap<String, GioVolume> {
    let mut volumes = BTreeMap::new();
    let mut volume_label = None;
    let lines = list.lines().map(str::trim);
    for line in lines {
        if let Some(label) = line
            .strip_prefix("Volume(")
            .and_then(|line| line.split_once("): "))
        {
            volume_label = Some(label.1);
        } else if let Some(uri) = line.strip_prefix("activation_root=") {
            if let Some(label) = volume_label.take()
                && let Some(uri) = phone_uri(uri)
            {
                volumes.entry(uri).or_insert_with(|| GioVolume {
                    label: label.to_owned(),
                    mounted: false,
                });
            }
        } else if let Some((label, uri)) = line
            .strip_prefix("Mount(")
            .and_then(|line| line.split_once("): "))
            .and_then(|(_, mount)| mount.rsplit_once(" -> "))
        {
            if let Some(uri) = phone_uri(uri) {
                let entry = volumes.entry(uri).or_insert_with(|| GioVolume {
                    label: label.to_owned(),
                    mounted: true,
                });
                entry.mounted = true;
            }
        } else if line.starts_with("Drive(") {
            volume_label = None;
        }
    }
    volumes
}

fn phone_uri(uri: &str) -> Option<String> {
    let (scheme, address) = uri.split_once("://")?;
    if !matches!(scheme, "mtp" | "afc") || address.is_empty() {
        return None;
    }
    Some(format!("{}/", uri.trim_end_matches('/')))
}

fn local_path(uri: &str) -> Option<PathBuf> {
    let info = run(&["info", "--attributes=standard::type", "--", uri]).ok()?;
    let path = PathBuf::from(
        info.lines()
            .find_map(|line| line.strip_prefix("local path: "))?,
    );
    path.is_absolute().then_some(path)
}

pub(super) fn mount(uri: &str) -> io::Result<PathBuf> {
    let result = run(&["mount", "--", uri]);
    if let Some((_, path)) = fuse_mounts().remove(&phone_key(uri)) {
        return Ok(path);
    }
    if let Some(path) = local_path(uri) {
        return Ok(path);
    }
    result?;
    Err(io::Error::other(
        "手机已挂载，但没有可浏览的本地路径，请检查 gvfs-fuse",
    ))
}

pub(super) fn unmount(uri: &str) -> io::Result<()> {
    run(&["mount", "--unmount", "--", uri]).map(drop)
}

fn run(args: &[&str]) -> io::Result<String> {
    let output = Command::new("gio")
        .env("LC_ALL", "C.UTF-8")
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::{fuse_phone_uri, parse_volumes, phone_key};

    #[test]
    fn matches_fuse_and_gio_spellings_of_a_phone() {
        let fuse =
            fuse_phone_uri("mtp:host=%5Busb%3A001%2C005%5D").expect("MTP 挂载名应解析为 URI");
        assert_eq!(phone_key(&fuse), phone_key("mtp://[usb:001,005]/"));
        assert!(fuse_phone_uri("smb-share:server=nas,share=doc").is_none());
    }

    #[test]
    fn discovers_mounted_and_unmounted_phones_without_duplicate_rows() {
        let volumes = parse_volumes(
            "Drive(0): Device\n  Volume(0): Pixel\n    activation_root=mtp://Pixel/\n  Mount(0): Pixel -> mtp://Pixel\nVolume(1): iPhone\n  activation_root=afc://iPhone/\nMount(1): Share -> smb://nas/share\n",
        );
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes["mtp://Pixel/"].label, "Pixel");
        assert!(
            volumes["mtp://Pixel/"].mounted,
            "GIO 同时列出了 Pixel 的挂载项"
        );
        assert_eq!(volumes["afc://iPhone/"].label, "iPhone");
        assert!(
            !volumes["afc://iPhone/"].mounted,
            "GIO 只列出了 iPhone 的卷"
        );
    }
}
