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
    match kind {
        EntryKind::Directory => (IconName::Folder, "文件夹", false),
        EntryKind::File => {
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            match extension.to_ascii_lowercase().as_str() {
                "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpeg" | "mpg"
                | "mts" | "m2ts" | "3gp" => (IconName::Film, "视频", true),
                "mp3" | "m4a" | "flac" | "ogg" | "oga" | "opus" | "wav" | "aac" | "aif"
                | "aiff" | "wma" => (IconName::FileMusic, "音频", true),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "avif" => {
                    (IconName::Image, "图片", true)
                }
                "svg" | "ico" | "heic" | "heif" => (IconName::Image, "图片", false),
                "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst" | "tgz" => {
                    (IconName::FileArchive, "压缩包", false)
                }
                "pdf" => (IconName::FileType, "PDF 文档", false),
                "doc" | "docx" | "docm" | "dot" | "dotx" | "dotm" => {
                    (IconName::FileText, "Word 文档", false)
                }
                "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "ppsm" | "pot" | "potx" | "potm" => {
                    (IconName::Presentation, "PowerPoint 演示文稿", false)
                }
                "xls" | "xlsx" | "xlsm" | "xlsb" | "xlt" | "xltx" | "xltm" => {
                    (IconName::FileSpreadsheet, "Excel 工作簿", false)
                }
                "csv" => (IconName::Table2, "CSV 表格", false),
                "tsv" => (IconName::Table2, "TSV 表格", false),
                "odt" | "ott" | "wps" | "wpt" | "pages" | "rtf" => {
                    (IconName::FileText, "文字文档", false)
                }
                "odp" | "otp" | "dps" | "dpt" | "key" => {
                    (IconName::Presentation, "演示文稿", false)
                }
                "ods" | "ots" | "et" | "ett" | "numbers" => {
                    (IconName::FileSpreadsheet, "电子表格", false)
                }
                "txt" | "log" | "md" | "markdown" => (IconName::File, "文本文件", false),
                "epub" | "mobi" | "azw" | "azw3" => (IconName::BookOpen, "电子书", false),
                "eml" | "msg" => (IconName::Mail, "邮件", false),
                "ics" => (IconName::Calendar, "日历文件", false),
                "vcf" => (IconName::Contact, "联系人文件", false),
                "rs" | "py" | "js" | "ts" | "html" | "css" | "json" | "toml" | "yml" | "yaml"
                | "sh" => (IconName::FileCode, "代码", false),
                _ => (IconName::File, "文件", false),
            }
        }
        EntryKind::Symlink => (IconName::Link, "符号链接", false),
        EntryKind::Other => (IconName::File, "其他", false),
        EntryKind::Missing => (IconName::File, "不可用", false),
    }
}
