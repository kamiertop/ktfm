use std::path::PathBuf;

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

pub fn entry_appearance(kind: EntryKind) -> (IconName, &'static str) {
    match kind {
        EntryKind::Directory => (IconName::Folder, "文件夹"),
        EntryKind::File => (IconName::File, "文件"),
        EntryKind::Symlink => (IconName::Link, "符号链接"),
        EntryKind::Other => (IconName::File, "其他"),
        EntryKind::Missing => (IconName::File, "不可用"),
    }
}
