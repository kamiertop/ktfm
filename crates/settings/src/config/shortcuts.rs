use serde::{Deserialize, Serialize};

/// 全部可配置快捷键，使用 GPUI 的按键格式存储。
///
/// 每个字段对应 gui crate `keymap::SHORTCUTS` 注册表中的一条记录；
/// 空字符串表示未绑定。容器级 `serde(default)` 让旧配置文件缺字段时
/// 自动落到默认值，新增字段无需迁移。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutConfig {
    pub new_tab: String,
    pub close_tab: String,
    pub split_pane: String,
    pub split_down: String,
    pub split_left: String,
    pub split_up: String,
    pub pane_left: String,
    pub pane_right: String,
    pub pane_up: String,
    pub pane_down: String,
    pub close_pane: String,
    pub open_terminal: String,
    pub open_preferences: String,
    pub zoom_in: String,
    pub zoom_out: String,
    pub select_all: String,
    pub copy: String,
    pub cut: String,
    pub paste: String,
    pub rename: String,
    pub trash: String,
    /// 新建文件夹，默认不绑定。
    pub new_folder: String,
    /// 新建文档，默认不绑定。
    pub new_document: String,
    /// 当前文件夹属性，默认不绑定。
    pub properties: String,
    /// 内嵌终端选区复制。
    pub terminal_copy: String,
    /// 内嵌终端剪贴板粘贴。
    pub terminal_paste: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            new_tab: "ctrl-t".into(),
            close_tab: "ctrl-w".into(),
            split_pane: "ctrl-right".into(),
            split_down: "ctrl-down".into(),
            split_left: "ctrl-left".into(),
            split_up: "ctrl-up".into(),
            pane_left: "alt-left".into(),
            pane_right: "alt-right".into(),
            pane_up: "alt-up".into(),
            pane_down: "alt-down".into(),
            close_pane: "shift-f3".into(),
            open_terminal: "ctrl-`".into(),
            open_preferences: "ctrl-p".into(),
            zoom_in: "ctrl-=".into(),
            zoom_out: "ctrl--".into(),
            select_all: "ctrl-a".into(),
            copy: "ctrl-c".into(),
            cut: "ctrl-x".into(),
            paste: "ctrl-v".into(),
            rename: "f2".into(),
            trash: "delete".into(),
            new_folder: String::new(),
            new_document: String::new(),
            properties: String::new(),
            // 与文件区的复制/粘贴同键：两者生效范围不相交（见 keymap 注册表）。
            terminal_copy: "ctrl-c".into(),
            terminal_paste: "ctrl-v".into(),
        }
    }
}
