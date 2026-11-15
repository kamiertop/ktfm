use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use super::{APP_NAME, AppConfig, star::Star, tab::Session};
use crate::{Error, Result};

const FILE_NAME: &str = "workspace.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub stars: Star,
    pub session: Session,
}

impl Workspace {
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path()?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path()?)
    }

    fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", APP_NAME).ok_or(Error::NoConfigDir)?;
        Ok(dirs.config_dir().join(FILE_NAME))
    }

    fn load_from(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => Ok(toml::from_str(&content)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        AppConfig::atomic_write(path, &toml::to_string_pretty(self)?)
    }
}

/// 普通路径用字符串存储，非 UTF-8 路径保留操作系统原始编码。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "PathEncoding", into = "PathEncoding")]
pub struct SavedPath(pub PathBuf);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PathEncoding {
    Text(String),
    Native(OsString),
}

impl From<SavedPath> for PathEncoding {
    fn from(path: SavedPath) -> Self {
        match path.0.into_os_string().into_string() {
            Ok(text) => Self::Text(text),
            Err(native) => Self::Native(native),
        }
    }
}

impl From<PathEncoding> for SavedPath {
    fn from(encoding: PathEncoding) -> Self {
        Self(match encoding {
            PathEncoding::Text(text) => text.into(),
            PathEncoding::Native(native) => native.into(),
        })
    }
}

impl std::borrow::Borrow<Path> for SavedPath {
    fn borrow(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tab::{Location, TabItem};

    use uuid::Uuid;

    #[test]
    fn restores_favorites_and_active_tab() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("ktfm-test-workspace-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let result = (|| {
            let file = dir.join(FILE_NAME);
            assert!(Workspace::load_from(&file)?.session.tabs.is_empty());

            let mut workspace = Workspace::default();
            let directory = dir.join("照片");
            let favorite = workspace.stars.add(directory.clone(), None);
            assert_eq!(workspace.stars.add(directory.clone(), None), favorite);
            let tag = Uuid::new_v4();
            let id = Uuid::new_v4();
            workspace.session.tabs = vec![
                TabItem {
                    id: Uuid::new_v4(),
                    location: Location::directory(directory.clone()),
                },
                TabItem {
                    id,
                    location: Location::Tag { id: tag },
                },
            ];
            workspace.session.active = Some(id);
            workspace.save_to(&file)?;

            let restored = Workspace::load_from(&file)?;
            assert_eq!(restored.stars.find(&directory).unwrap().id, favorite);
            assert_eq!(restored.session.active, Some(id));
            assert_eq!(restored.session.tabs[1].location, Location::Tag { id: tag });
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_paths_in_toml() -> Result<()> {
        use std::os::unix::ffi::OsStringExt;

        let raw = PathBuf::from(OsString::from_vec(b"/tmp/non-utf8-\xff".to_vec()));
        let mut workspace = Workspace::default();
        workspace.stars.add(raw.clone(), None);
        let content = toml::to_string_pretty(&workspace)?;
        let restored: Workspace = toml::from_str(&content)?;
        assert!(restored.stars.find(&raw).is_some());
        Ok(())
    }
}
