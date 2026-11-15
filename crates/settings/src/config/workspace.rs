use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use super::{APP_NAME, AppConfig, bookmark::Bookmarks, tab::Session};
use crate::{Error, Result};

const FILE_NAME: &str = "workspace.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    #[serde(alias = "stars")]
    pub bookmarks: Bookmarks,
    pub session: Session,
}

impl Workspace {
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path()?)
    }

    /// 在跨进程锁内读取最新工作区并更新，锁文件本身不会被原子替换。
    pub fn update(change: impl FnOnce(&mut Self)) -> Result<Self> {
        Self::update_at(&Self::path()?, change)
    }

    fn update_at(path: &Path, change: impl FnOnce(&mut Self)) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _lock = crate::file_lock::acquire(&path.with_extension("lock"))?;
        let mut workspace = Self::load_from(path)?;
        change(&mut workspace);
        workspace.save_to(path)?;
        Ok(workspace)
    }

    pub fn path() -> Result<PathBuf> {
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
    use crate::config::bookmark::BookmarkKind;
    use crate::config::tab::{Location, TabItem};

    use uuid::Uuid;

    #[test]
    fn concurrent_workspace_updates_preserve_bookmarks_and_session() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("ktfm-test-workspace-lock-{}", Uuid::new_v4()));
        fs::create_dir(&dir)?;
        let result = (|| {
            let file = dir.join(FILE_NAME);
            let start = std::sync::Barrier::new(2);
            let active = Uuid::new_v4();
            std::thread::scope(|scope| {
                let first = scope.spawn(|| {
                    start.wait();
                    Workspace::update_at(&file, |workspace| {
                        workspace
                            .bookmarks
                            .add(dir.join("a"), BookmarkKind::Directory);
                    })
                });
                start.wait();
                Workspace::update_at(&file, |workspace| {
                    workspace.session.active = Some(active);
                    workspace
                        .bookmarks
                        .add(dir.join("b"), BookmarkKind::Directory);
                })?;
                first.join().expect("工作区写入线程正常结束")?;
                Ok::<_, Error>(())
            })?;

            let workspace = Workspace::load_from(&file)?;
            assert_eq!(workspace.session.active, Some(active));
            assert_eq!(workspace.bookmarks.items().len(), 2);
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }

    #[test]
    fn restores_bookmarks_and_active_tab() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("ktfm-test-workspace-{}", std::process::id()));
        fs::create_dir(&dir)?;
        let result = (|| {
            let file = dir.join(FILE_NAME);
            assert!(Workspace::load_from(&file)?.session.tabs.is_empty());

            let mut workspace = Workspace::default();
            let directory = dir.join("照片");
            let favorite = workspace
                .bookmarks
                .add(directory.clone(), BookmarkKind::Directory);
            assert_eq!(
                workspace
                    .bookmarks
                    .add(directory.clone(), BookmarkKind::Directory),
                favorite
            );
            let file_path = dir.join("照片.jpg");
            let bookmark = workspace
                .bookmarks
                .add(file_path.clone(), BookmarkKind::File);
            workspace.bookmarks.rename(bookmark, Some("封面".into()));
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
            assert_eq!(restored.bookmarks.find(&directory).unwrap().id, favorite);
            let file_bookmark = restored.bookmarks.find(&file_path).unwrap();
            assert_eq!(file_bookmark.kind, BookmarkKind::File);
            assert_eq!(file_bookmark.label.as_deref(), Some("封面"));
            assert_eq!(restored.session.active, Some(id));
            assert_eq!(restored.session.tabs[1].location, Location::Tag { id: tag });
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }

    #[test]
    fn reads_legacy_stars_as_directory_bookmarks() -> Result<()> {
        let id = Uuid::new_v4();
        let workspace: Workspace = toml::from_str(&format!(
            "[[stars.items]]\nid = \"{id}\"\npath = \"/tmp/example\"\n"
        ))?;
        let bookmark = workspace.bookmarks.get(id).unwrap();
        assert_eq!(bookmark.path.0, Path::new("/tmp/example"));
        assert_eq!(bookmark.kind, BookmarkKind::Directory);
        assert!(bookmark.label.is_none());
        assert!(toml::to_string_pretty(&workspace)?.contains("[[bookmarks.items]]"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_paths_in_toml() -> Result<()> {
        use std::os::unix::ffi::OsStringExt;

        let raw = PathBuf::from(OsString::from_vec(b"/tmp/non-utf8-\xff".to_vec()));
        let mut workspace = Workspace::default();
        workspace.bookmarks.add(raw.clone(), BookmarkKind::File);
        let content = toml::to_string_pretty(&workspace)?;
        let restored: Workspace = toml::from_str(&content)?;
        assert!(restored.bookmarks.find(&raw).is_some());
        Ok(())
    }
}
