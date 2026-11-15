use serde::{Deserialize, Serialize};

/// 文件管理器快捷键，使用 GPUI 的按键格式存储。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutConfig {
    pub new_tab: String,
    pub close_tab: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            new_tab: "ctrl-t".into(),
            close_tab: "ctrl-w".into(),
        }
    }
}
