use std::path::Path;

use vfs::EntryKind;

#[derive(Clone, Copy)]
pub enum FileType {
    Folder,
    Video,
    Audio,
    ImagePreview,
    Image,
    Archive,
    Pdf,
    Word,
    PowerPoint,
    Excel,
    Csv,
    Tsv,
    Document,
    Presentation,
    Spreadsheet,
    Text,
    Book,
    Mail,
    Calendar,
    Contact,
    Code,
    File,
    Symlink,
    Other,
    Missing,
}

impl FileType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "文件夹",
            Self::Video => "视频",
            Self::Audio => "音频",
            Self::ImagePreview | Self::Image => "图片",
            Self::Archive => "压缩包",
            Self::Pdf => "PDF 文档",
            Self::Word => "Word 文档",
            Self::PowerPoint => "PowerPoint 演示文稿",
            Self::Excel => "Excel 工作簿",
            Self::Csv => "CSV 表格",
            Self::Tsv => "TSV 表格",
            Self::Document => "文字文档",
            Self::Presentation => "演示文稿",
            Self::Spreadsheet => "电子表格",
            Self::Text => "文本文件",
            Self::Book => "电子书",
            Self::Mail => "邮件",
            Self::Calendar => "日历文件",
            Self::Contact => "联系人文件",
            Self::Code => "代码",
            Self::File => "文件",
            Self::Symlink => "符号链接",
            Self::Other => "其他",
            Self::Missing => "不可用",
        }
    }

    pub fn previewable(self) -> bool {
        matches!(self, Self::Video | Self::Audio | Self::ImagePreview)
    }
}

pub fn file_type(kind: EntryKind, path: &Path) -> FileType {
    match kind {
        EntryKind::Directory => FileType::Folder,
        EntryKind::File => {
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            match extension.to_ascii_lowercase().as_str() {
                "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpeg" | "mpg"
                | "mts" | "m2ts" | "3gp" => FileType::Video,
                "mp3" | "m4a" | "flac" | "ogg" | "oga" | "opus" | "wav" | "aac" | "aif"
                | "aiff" | "wma" => FileType::Audio,
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "avif"
                | "svg" => FileType::ImagePreview,
                "ico" | "heic" | "heif" => FileType::Image,
                "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst" | "tgz" => {
                    FileType::Archive
                }
                "pdf" => FileType::Pdf,
                "doc" | "docx" | "docm" | "dot" | "dotx" | "dotm" => FileType::Word,
                "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "ppsm" | "pot" | "potx" | "potm" => {
                    FileType::PowerPoint
                }
                "xls" | "xlsx" | "xlsm" | "xlsb" | "xlt" | "xltx" | "xltm" => FileType::Excel,
                "csv" => FileType::Csv,
                "tsv" => FileType::Tsv,
                "odt" | "ott" | "wps" | "wpt" | "pages" | "rtf" => FileType::Document,
                "odp" | "otp" | "dps" | "dpt" | "key" => FileType::Presentation,
                "ods" | "ots" | "et" | "ett" | "numbers" => FileType::Spreadsheet,
                "txt" | "log" | "md" | "markdown" => FileType::Text,
                "epub" | "mobi" | "azw" | "azw3" => FileType::Book,
                "eml" | "msg" => FileType::Mail,
                "ics" => FileType::Calendar,
                "vcf" => FileType::Contact,
                "rs" | "py" | "js" | "ts" | "html" | "css" | "json" | "toml" | "yml" | "yaml"
                | "sh" => FileType::Code,
                _ => FileType::File,
            }
        }
        EntryKind::Symlink => FileType::Symlink,
        EntryKind::Other => FileType::Other,
        EntryKind::Missing => FileType::Missing,
    }
}
