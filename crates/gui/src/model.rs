use std::path::{Path, PathBuf};

use directories::{BaseDirs, UserDirs};
use gpui_kit::assets::IconName;
use vfs::EntryKind;

/// 首次启动播种进侧栏书签的 XDG 常用目录。主目录不在此列，
/// 它固定显示在侧栏顶部。不带固定显示名：名称由路径本身决定，
/// 跟随用户 user-dirs 配置的语言。
pub fn default_bookmarks() -> Vec<PathBuf> {
    let user_dirs = UserDirs::new();
    let user = user_dirs.as_ref();
    let directory = |dir: fn(&UserDirs) -> Option<&Path>| user.and_then(dir).map(PathBuf::from);
    [
        directory(UserDirs::desktop_dir),
        directory(UserDirs::document_dir),
        directory(UserDirs::download_dir),
        directory(UserDirs::picture_dir),
        directory(UserDirs::audio_dir),
        directory(UserDirs::video_dir),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub fn home_directory() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

pub fn entry_appearance(kind: EntryKind, path: &Path) -> (IconName, &'static str, bool) {
    use daemon::file_type::{FileType, file_type};

    let kind = file_type(kind, path);
    let icon = match kind {
        FileType::Folder => IconName::Folder,
        FileType::Video => IconName::Film,
        FileType::Audio => IconName::FileMusic,
        FileType::ImagePreview | FileType::Image => IconName::Image,
        FileType::Archive => IconName::FileArchive,
        FileType::Pdf => IconName::FileType,
        FileType::Word | FileType::Document => IconName::FileText,
        FileType::PowerPoint | FileType::Presentation => IconName::Presentation,
        FileType::Excel | FileType::Spreadsheet => IconName::FileSpreadsheet,
        FileType::Csv | FileType::Tsv => IconName::Table2,
        FileType::Text | FileType::File | FileType::Other | FileType::Missing => IconName::File,
        FileType::Book => IconName::BookOpen,
        FileType::Mail => IconName::Mail,
        FileType::Calendar => IconName::Calendar,
        FileType::Contact => IconName::Contact,
        FileType::Code => IconName::FileCode,
        FileType::Symlink => IconName::Link,
    };
    (icon, kind.label(), kind.previewable())
}
