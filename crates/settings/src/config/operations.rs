use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct OperationsConfig {
    /// 按下Delete键时的删除模式
    pub delete_mode: DeleteMode,

    /// 物理删除时是否弹出确认框，默认为 true
    pub confirm_permanent_delete: bool,

    /// 清空回收站时是否弹出确认框，默认为 true
    pub confirm_empty_trash: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    /// 移动到回收站，可恢复
    #[default]
    Trash,

    /// 物理删除，不可逆
    Permanent,
}

impl Default for OperationsConfig {
    fn default() -> Self {
        OperationsConfig {
            delete_mode: DeleteMode::default(),
            confirm_permanent_delete: true,
            confirm_empty_trash: true,
        }
    }
}