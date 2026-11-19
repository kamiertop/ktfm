use std::ffi::OsStr;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod desktop {
    use std::path::Path;
    use std::process::Command;
    use std::sync::OnceLock;

    use xdg_mime::SharedMimeInfo;

    use super::*;

    pub fn icon_names(name: &OsStr) -> (Option<String>, Vec<String>) {
        static MIME: OnceLock<SharedMimeInfo> = OnceLock::new();
        let database = MIME.get_or_init(SharedMimeInfo::new);
        // 目录列表使用文件名匹配，避免为了绘制图标打开每个文件。
        database
            .get_mime_types_from_file_name(&name.to_string_lossy())
            .into_iter()
            .next()
            .map(|mime| {
                (
                    Some(mime.essence_str().to_owned()),
                    database.lookup_icon_names(&mime),
                )
            })
            .unwrap_or_default()
    }

    fn theme_from_ini(path: &Path, section: &str, key: &str) -> Option<String> {
        let contents = std::fs::read_to_string(path).ok()?;
        let mut in_section = false;
        for line in contents.lines().map(str::trim) {
            if let Some(header) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                in_section = header == section;
            } else if in_section
                && let Some((name, value)) = line.split_once('=')
                && name.trim() == key
                && !value.trim().is_empty()
            {
                return Some(value.trim().to_owned());
            }
        }
        None
    }

    fn theme() -> String {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        if desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("KDE"))
        {
            return config
                .and_then(|dir| theme_from_ini(&dir.join("kdeglobals"), "Icons", "Theme"))
                .unwrap_or_else(|| "breeze".into());
        }
        if desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("GNOME"))
            && let Ok(output) = Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "icon-theme"])
                .output()
            && output.status.success()
            && let Ok(value) = std::str::from_utf8(&output.stdout)
            && let Some(name) = value
                .trim()
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
            && !name.is_empty()
        {
            return name.to_owned();
        }
        config
            .and_then(|dir| {
                ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
                    .into_iter()
                    .find_map(|file| {
                        theme_from_ini(&dir.join(file), "Settings", "gtk-icon-theme-name")
                    })
            })
            .unwrap_or_else(|| "Adwaita".into())
    }

    pub fn themed_icon(name: &str, size: u16, scale: u16) -> Option<PathBuf> {
        static THEME: OnceLock<String> = OnceLock::new();
        freedesktop_icons::lookup(name)
            .with_theme(THEME.get_or_init(theme))
            .with_size(size)
            .with_scale(scale)
            .find()
    }
}

#[cfg(target_os = "linux")]
pub use desktop::{icon_names, themed_icon};

#[cfg(not(target_os = "linux"))]
pub fn icon_names(_: &OsStr) -> (Option<String>, Vec<String>) {
    (None, Vec::new())
}

#[cfg(not(target_os = "linux"))]
pub fn themed_icon(_: &str, _: u16, _: u16) -> Option<PathBuf> {
    None
}
