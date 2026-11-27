use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{fs, io};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use super::bookmark::Bookmarks;
use super::file_columns::FileListColumns;
use super::tab::Session;
use super::{APP_NAME, AppConfig};
use crate::{Error, Result};

const FILE_NAME: &str = "workspace.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    #[serde(alias = "stars")]
    pub bookmarks: Bookmarks,
    pub session: Session,
    pub file_list_columns: FileListColumns,
    /// 上次记住的窗口大小，下次启动时恢复
    pub window_size: Option<WindowSize>,
    /// 是否已把 XDG 常用目录（不含主目录，主目录固定在侧栏顶部）播种
    /// 为书签。只播种一次，之后用户的删除不会被下次启动重新加回。
    pub default_directories_seeded: bool,
}

/// 记住的窗口大小（逻辑像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

impl WindowSize {
    /// 丢弃 NaN/无穷/过小等异常值，避免把不可用的尺寸写进工作区。
    pub fn sanitize(self) -> Option<Self> {
        (self.width.is_finite() && self.height.is_finite() && self.width >= 1. && self.height >= 1.)
            .then_some(self)
    }
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
    use crate::config::tab::{Location, SplitLayout, TabItem};

    use uuid::Uuid;

    #[test]
    fn workspace_without_seed_flag_reads_as_unseeded() {
        // 没有播种标记的工作区视为未播种；置位后随文件持久化。
        let mut workspace: Workspace = toml::from_str("").unwrap();
        assert!(!workspace.default_directories_seeded);
        workspace.default_directories_seeded = true;
        let text = toml::to_string_pretty(&workspace).unwrap();
        assert!(text.contains("default_directories_seeded = true"));
    }

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
    fn restores_third_pane_and_its_active_position() -> Result<()> {
        let id = Uuid::new_v4();
        let content = format!(
            "[session.split]\ntabs = []\nright_active = true\nratio = 0.3\nlayout = \"stacked\"\n\n\
             [session.split.third]\nactive = \"{id}\"\nactive_index = 2\nmiddle_ratio = 0.4\n\n\
             [[session.split.third.tabs]]\nid = \"{id}\"\nlocation = {{ kind = \"directory\", path = \"/tmp\" }}\n",
        );
        let workspace: Workspace = toml::from_str(&content)?;
        let restored: Workspace = toml::from_str(&toml::to_string_pretty(&workspace)?)?;
        assert_eq!(restored.session.view_mode, None);
        let split = restored.session.split.unwrap();
        assert!(
            crate::config::tab::GridSession::from(split.clone())
                .panes
                .iter()
                .all(|pane| pane.view_mode.is_none())
        );
        assert_eq!(split.layout, SplitLayout::Stacked);
        assert_eq!(split.ratio, 0.3);
        let third = split.third.unwrap();
        assert_eq!(third.active, Some(id));
        assert_eq!(third.active_index, 2);
        assert_eq!(third.middle_ratio, 0.4);
        assert_eq!(third.tabs[0].location, Location::directory("/tmp".into()));
        Ok(())
    }

    #[test]
    fn restores_six_panes_with_independent_column_splits() -> Result<()> {
        use crate::config::tab::{GridSession, PaneColumn, PaneSession};
        use crate::config::view::ViewMode;

        let pane = |index| {
            let id = Uuid::new_v4();
            PaneSession {
                tabs: vec![TabItem {
                    id,
                    location: Location::directory(format!("/pane/{index}").into()),
                }],
                active: Some(id),
                zoom_step: (index as i8 - 2).min(1),
                view_mode: Some(if index == 0 {
                    ViewMode::Grid
                } else {
                    ViewMode::List
                }),
            }
        };
        let first = pane(0);
        let mut workspace = Workspace::default();
        workspace.session.tabs = first.tabs;
        workspace.session.active = first.active;
        workspace.session.view_mode = first.view_mode;
        workspace.session.zoom_step = first.zoom_step;
        workspace.session.grid = Some(GridSession {
            columns: [(2, [2, 1], 0.35), (2, [1, 1], 0.5), (1, [1, 1], 0.75)]
                .into_iter()
                .map(|(rows, row_panes, top_ratio)| PaneColumn {
                    rows,
                    row_panes,
                    width_ratio: 1. / 3.,
                    top_ratio,
                    ..PaneColumn::default()
                })
                .collect(),
            panes: (1..6).map(pane).collect(),
            active_index: 5,
        });

        let restored: Workspace = toml::from_str(&toml::to_string_pretty(&workspace)?)?;
        assert_eq!(restored.session.active, first.active);
        assert_eq!(restored.session.view_mode, Some(ViewMode::Grid));
        assert_eq!(restored.session.zoom_step, -2);
        let grid = restored.session.grid.unwrap();
        assert_eq!(grid.active_index, 5);
        assert_eq!(
            grid.columns
                .iter()
                .map(|column| column.top_ratio)
                .collect::<Vec<_>>(),
            [0.35, 0.5, 0.75]
        );
        assert_eq!(grid.columns[0].row_panes, [2, 1]);
        assert_eq!(grid.columns[0].pane_count(), 3);
        assert_eq!(grid.panes.len(), 5);
        assert_eq!(grid.panes[4].view_mode, Some(ViewMode::List));
        assert_eq!(grid.panes[4].zoom_step, 1);
        assert_eq!(
            grid.panes[4].tabs[0].location,
            Location::directory("/pane/5".into())
        );
        Ok(())
    }

    #[test]
    fn window_size_roundtrips_and_defaults_to_none() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("ktfm-test-window-size-{}", Uuid::new_v4()));
        fs::create_dir(&dir)?;
        let result = (|| {
            let file = dir.join(FILE_NAME);
            let workspace = Workspace {
                window_size: Some(WindowSize {
                    width: 1440.,
                    height: 860.,
                }),
                ..Workspace::default()
            };
            workspace.save_to(&file)?;

            let restored = Workspace::load_from(&file)?;
            assert_eq!(
                restored.window_size,
                Some(WindowSize {
                    width: 1440.,
                    height: 860.,
                })
            );

            // 旧版工作区文件没有 window_size 字段时应回退为 None
            fs::write(&file, "[session]\ntabs = []\n")?;
            assert_eq!(Workspace::load_from(&file)?.window_size, None);
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }

    #[test]
    fn window_size_sanitizes_invalid_values() {
        assert_eq!(
            WindowSize {
                width: 800.,
                height: 600.
            }
            .sanitize(),
            Some(WindowSize {
                width: 800.,
                height: 600.
            })
        );
        assert_eq!(
            WindowSize {
                width: f32::NAN,
                height: 600.
            }
            .sanitize(),
            None
        );
        assert_eq!(
            WindowSize {
                width: 0.,
                height: 600.
            }
            .sanitize(),
            None,
            "过小的尺寸不应写入工作区"
        );
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
