use std::path::PathBuf;

pub enum AppCommand {
    // 导航类
    NavigateTo(PathBuf),
    NavigateUp,
    NavigateBack,
    NavigateForward,
    RefreshCurrentDir,

    // 激活/打开目标条目
    ActivateEntry(PathBuf),
    
    /// 创建新标签页
    CreateNewTab(PathBuf),

    OpenNewWindow(PathBuf),
    
    // 标签页操作
    DuplicateCurrentTab,
    CloseCurrentTab,
}