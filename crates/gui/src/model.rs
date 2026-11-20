use std::path::{Path, PathBuf};

use directories::{BaseDirs, UserDirs};
use gpui_kit::assets::IconName;
use vfs::EntryKind;

pub struct SidebarItem {
    pub name: &'static str,
    pub icon: IconName,
    pub path: Option<PathBuf>,
}

pub fn default_favorites(home: Option<PathBuf>) -> Vec<SidebarItem> {
    let user_dirs = UserDirs::new();
    let user = user_dirs.as_ref();
    let item = |name, icon, path| SidebarItem { name, icon, path };

    vec![
        item("主文件夹", IconName::House, home),
        item(
            "桌面",
            IconName::Monitor,
            user.and_then(UserDirs::desktop_dir).map(Into::into),
        ),
        item(
            "文档",
            IconName::Folder,
            user.and_then(UserDirs::document_dir).map(Into::into),
        ),
        item(
            "下载",
            IconName::Download,
            user.and_then(UserDirs::download_dir).map(Into::into),
        ),
        item(
            "图片",
            IconName::Images,
            user.and_then(UserDirs::picture_dir).map(Into::into),
        ),
        item(
            "音乐",
            IconName::Music,
            user.and_then(UserDirs::audio_dir).map(Into::into),
        ),
        item(
            "视频",
            IconName::Film,
            user.and_then(UserDirs::video_dir).map(Into::into),
        ),
    ]
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
