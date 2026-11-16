use crate::command::AppCommand;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct ActionConfig {
    /// 空白区域双击的操作
    pub blank_double_click: BlankDoubleClickAction,
    /// 鼠标单击空白区域的操作
    pub blank_single_click: BlankSingleClickAction,
    /// 鼠标中键点击标签页的操作
    pub middle_click_tab: TabMiddleClickAction,
    /// 鼠标中键点击文件夹的操作
    pub middle_click_folder: FolderMiddleClickAction,

    /// Ctrl+鼠标滚轮的操作
    pub ctrl_wheel: CtrlWheelAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FolderMiddleClickAction {
    /// 在新标签页中打开文件夹
    #[default]
    OpenInNewTab,
    /// 无操作
    None,
    /// 在新窗口中打开文件夹
    OpenInNewWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlankSingleClickAction {
    /// 清除选中状态
    #[default]
    ClearSelection,
    /// 无操作
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabMiddleClickAction {
    /// 无操作
    None,
    /// 关闭标签页
    #[default]
    CloseTab,
    /// 复制标签页
    CopyTab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlankDoubleClickAction {
    /// 无操作
    None,
    /// 刷新当前目录
    Refresh,
    /// 返回上一级目录
    #[default]
    GoUp,
}

/// Ctrl+鼠标滚轮操作
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CtrlWheelAction {
    /// 放大或缩小
    #[default]
    #[serde(alias = "zome_view")]
    ZoomView,
    /// 无操作
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct DragConfig {
    /// 是否启用拖拽功能，默认开启
    pub enable: bool,
    pub action: DragAction,
    pub conflict_strategy: DragConflictStrategy,
    /// 拖拽时悬停在文件夹上的操作
    pub hover_folder_action: DragHoverFolderAction,
    pub hover_open_delay: u32, // 悬停打开文件夹的延迟时间，单位毫秒
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DragHoverFolderAction {
    None,
    /// 在悬停一段时间后自动打开文件夹
    #[default]
    Delayed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DragConflictStrategy {
    /// 询问用户如何处理
    #[default]
    Ask,
    /// 自动覆盖
    Overwrite,
    /// 自动跳过
    Skip,
    /// 自动保留两个文件，然后重命名
    KeepBoth,
}
impl Default for DragConfig {
    fn default() -> Self {
        Self {
            enable: true,
            action: DragAction::default(),
            conflict_strategy: DragConflictStrategy::default(),
            hover_folder_action: DragHoverFolderAction::default(),
            hover_open_delay: 800, // 默认延迟800毫秒
        }
    }
}

/// 拖拽之后的放置策略
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DragAction {
    Ask,
    #[default]
    Auto,
    Copy,
    Move,
    Link,
}

impl BlankDoubleClickAction {
    pub fn to_command(self) -> Option<AppCommand> {
        match self {
            BlankDoubleClickAction::None => None,
            BlankDoubleClickAction::Refresh => Some(AppCommand::RefreshCurrentDir),
            BlankDoubleClickAction::GoUp => Some(AppCommand::NavigateUp),
        }
    }
}

impl TabMiddleClickAction {
    pub fn to_command(self) -> Option<AppCommand> {
        match self {
            TabMiddleClickAction::None => None,
            TabMiddleClickAction::CloseTab => Some(AppCommand::CloseCurrentTab),
            TabMiddleClickAction::CopyTab => Some(AppCommand::DuplicateCurrentTab),
        }
    }
}

impl FolderMiddleClickAction {
    pub fn to_command(self, path: &std::path::Path) -> Option<AppCommand> {
        match self {
            FolderMiddleClickAction::None => None,
            FolderMiddleClickAction::OpenInNewTab => {
                Some(AppCommand::CreateNewTab(path.to_path_buf()))
            }
            FolderMiddleClickAction::OpenInNewWindow => {
                Some(AppCommand::OpenNewWindow(path.to_path_buf()))
            }
        }
    }
}
