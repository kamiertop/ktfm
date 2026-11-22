use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use directories::ProjectDirs;
use gpui_kit::component::Icon;
use rust_embed::RustEmbed;

pub fn window_restore() -> Icon {
    Icon::default().data(include_bytes!("../../../assets/icons/window_restore.svg"))
}

#[derive(RustEmbed)]
#[folder = "../../assets/icons"]
struct BundledIcons;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum IconOverride {
    User(PathBuf),
    Bundled(String),
}

impl IconOverride {
    pub(crate) fn load(&self) -> Option<(&str, Cow<'static, [u8]>)> {
        match self {
            Self::User(path) => Some((
                path.extension()?.to_str()?,
                std::fs::read(path).ok()?.into(),
            )),
            Self::Bundled(name) => Some((
                Path::new(name).extension()?.to_str()?,
                BundledIcons::get(name)?.data,
            )),
        }
    }
}

#[derive(Default)]
struct Overrides {
    user: HashMap<String, IconOverride>,
    bundled: HashMap<String, IconOverride>,
}

impl Overrides {
    fn load() -> Self {
        let mut icons = Self::default();
        if let Some(dirs) = ProjectDirs::from("", "", "ktfm")
            && let Ok(entries) = std::fs::read_dir(dirs.config_dir().join("icons"))
        {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && entry.path().is_file()
                {
                    icons
                        .user
                        .insert(name.to_owned(), IconOverride::User(entry.path()));
                }
            }
        }
        for name in BundledIcons::iter() {
            if name == "window_restore.svg" {
                continue;
            }
            let name = name.into_owned();
            icons
                .bundled
                .insert(name.clone(), IconOverride::Bundled(name));
        }
        icons
    }
}

fn matching_icons(icons: &Overrides, name: &OsStr, mime: Option<&str>) -> Vec<IconOverride> {
    let name = name.to_string_lossy();
    let mut candidates: Vec<_> = name
        .match_indices('.')
        .filter_map(|(index, _)| {
            let suffix = &name[index + 1..];
            (!suffix.is_empty()).then(|| suffix.to_ascii_lowercase())
        })
        .collect();
    if let Some((category, subtype)) = mime.and_then(|mime| mime.split_once('/')) {
        candidates.push(format!("{category}-{subtype}").to_ascii_lowercase());
        candidates.push(subtype.to_ascii_lowercase());
    }

    // 用户目录优先，其次是随程序发布的图标。每层优先匹配更长的扩展名。
    let mut matches = Vec::new();
    for source in [&icons.user, &icons.bundled] {
        for candidate in &candidates {
            for extension in ["svg", "png"] {
                if let Some(icon) = source.get(&format!("{candidate}.{extension}"))
                    && !matches.contains(icon)
                {
                    matches.push(icon.clone());
                }
            }
        }
    }
    matches
}

pub(crate) fn file_icon_overrides(name: &OsStr, mime: Option<&str>) -> Vec<IconOverride> {
    static ICONS: OnceLock<Overrides> = OnceLock::new();
    matching_icons(ICONS.get_or_init(Overrides::load), name, mime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_config_format_icons_match_and_render() {
        let renderer = gpui_kit::SvgRenderer::new(std::sync::Arc::new(()));
        for (name, icon) in [
            ("a.json", "json.svg"),
            ("settings.ini", "ini.svg"),
            ("compose.yaml", "yaml.svg"),
            ("compose.yml", "yml.svg"),
        ] {
            let matched = file_icon_overrides(OsStr::new(name), None);
            assert_eq!(
                matched,
                [IconOverride::Bundled(icon.into())],
                "{name} 应命中自带图标"
            );
            let (extension, bytes) = matched[0].load().expect("图标应可加载");
            assert_eq!(extension, "svg");
            renderer
                .parse_svg(&bytes)
                .unwrap_or_else(|error| panic!("{icon} 解析失败: {error}"));
        }
    }

    #[test]
    fn overrides_match_compound_extensions_before_shorter_extensions_and_mime() {
        let custom = IconOverride::User("tar.zst.svg".into());
        let short = IconOverride::User("zst.png".into());
        let bundled = IconOverride::Bundled("zstd.png".into());
        let icons = Overrides {
            user: HashMap::from([
                ("tar.zst.svg".into(), custom.clone()),
                ("zst.png".into(), short.clone()),
            ]),
            bundled: HashMap::from([("zstd.png".into(), bundled.clone())]),
        };
        assert_eq!(
            matching_icons(
                &icons,
                OsStr::new("archive.TAR.ZST"),
                Some("application/zstd")
            ),
            [custom, short, bundled]
        );
    }
}
