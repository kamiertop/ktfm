use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FileDateFormat {
    #[default]
    Simple,
    Detailed,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct ViewConfig {
    pub view_mode: ViewMode,
    /// 是否显示隐藏文件，默认为 false
    pub show_hidden_files: bool,
    /// 切换标签页时保留搜索、选区和滚动位置。
    pub remember_tab_state: bool,
    /// 文件列表时间列共用的日期显示方式。
    pub file_date_format: FileDateFormat,
}

impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::default(),
            show_hidden_files: false,
            remember_tab_state: true,
            file_date_format: FileDateFormat::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// 列表视图
    #[default]
    List,
    /// 网格视图
    Grid,
}
