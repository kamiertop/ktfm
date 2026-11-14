use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct ViewConfig {
    pub view_mode: ViewMode,
    /// 是否显示隐藏文件，默认为 false
    pub show_hidden_files: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// 列表视图
    #[default]
    List,
    /// 网格视图
    Grid,
}
