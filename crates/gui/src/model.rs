use gpui_kit::assets::IconName;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub name: &'static str,
    pub icon: IconName,
}

pub const HOME: Location = Location {
    name: "主文件夹",
    icon: IconName::House,
};

pub const FOLDERS: &[Location] = &[
    HOME,
    Location {
        name: "桌面",
        icon: IconName::Monitor,
    },
    Location {
        name: "文档",
        icon: IconName::Folder,
    },
    Location {
        name: "下载",
        icon: IconName::Download,
    },
    Location {
        name: "图片",
        icon: IconName::Images,
    },
    Location {
        name: "音乐",
        icon: IconName::Music,
    },
    Location {
        name: "视频",
        icon: IconName::Film,
    },
];

pub const SIDEBAR: &[&[Location]] = &[
    FOLDERS,
    &[
        Location {
            name: "网络",
            icon: IconName::Network,
        },
        Location {
            name: "回收站",
            icon: IconName::Trash,
        },
    ],
    &[
        Location {
            name: "工作",
            icon: IconName::Tag,
        },
        Location {
            name: "个人",
            icon: IconName::Tag,
        },
    ],
    &[Location {
        name: "USB 存储",
        icon: IconName::Usb,
    }],
];

pub struct FileEntry {
    pub name: &'static str,
    pub icon: IconName,
    pub kind: &'static str,
    pub destination: Option<Location>,
}

pub fn entries(location: Location) -> Vec<FileEntry> {
    // 页面框架使用示例条目，后续由文件系统提供目录内容。
    if location == HOME {
        return FOLDERS[1..]
            .iter()
            .map(|&folder| FileEntry {
                name: folder.name,
                icon: IconName::Folder,
                kind: "文件夹",
                destination: Some(folder),
            })
            .collect();
    }

    let (name, icon, kind) = match location.name {
        "文档" | "工作" => ("项目笔记.md", IconName::FileText, "文档"),
        "下载" | "USB 存储" => ("备份.zip", IconName::FileArchive, "压缩包"),
        "图片" | "个人" => ("壁纸.png", IconName::Image, "图片"),
        "音乐" => ("试听.flac", IconName::FileMusic, "音频"),
        "视频" => ("录屏.mp4", IconName::Film, "视频"),
        _ => return Vec::new(),
    };
    vec![FileEntry {
        name,
        icon,
        kind,
        destination: None,
    }]
}
