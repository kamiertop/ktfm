//! 快捷键的唯一事实来源。
//!
//! [`SHORTCUTS`] 注册表把「配置字段 ⇄ 动作 ⇄ 生效范围 ⇄ 界面文案」钉在
//! 同一行：启动加载、保存校验、重新绑定和偏好设置页全部由这张表驱动。
//! 新增可配置快捷键只需给 [`settings::config::shortcuts::ShortcutConfig`]
//! 加一个字段，再在这里加一行。
//!
//! 键盘输入统一走 GPUI 的「键绑定 → 动作 → on_action」管线，不再在
//! `on_key_down` 里逐键嗅探；内嵌终端的剪贴板键也一样，只是动作由
//! 终端视图自己处理（见 `ShortcutScope::Terminal`）。

use gpui_kit::*;
use settings::config::shortcuts::ShortcutConfig;

use super::file_operations::FileAction;

// 标签页、终端与偏好设置等应用级动作。
actions!(
    browser,
    [
        NewTab,
        CloseTab,
        CancelCut,
        CancelPathEdit,
        SubmitPathEdit,
        NextPathSuggestion,
        PreviousPathSuggestion,
        OpenTerminal,
        OpenPreferences,
        ZoomIn,
        ZoomOut,
        ViewGrid,
        ViewList,
        SplitPane,
        SplitDown,
        SplitLeft,
        SplitUp,
        PaneLeft,
        PaneRight,
        PaneUp,
        PaneDown,
        ClosePane
    ]
);
// 内嵌终端聚焦时由终端视图处理的剪贴板动作。
actions!(browser, [TerminalCopy, TerminalPaste]);

const CANCEL_CUT_KEY: &str = "escape";
const CANCEL_CUT_LABEL: &str = "取消剪切";

pub(super) fn bind_fixed_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            CANCEL_CUT_KEY,
            CancelCut,
            Some("PendingFileCut && !EmbeddedTerminal && !PathEditor"),
        ),
        KeyBinding::new("escape", CancelPathEdit, Some("PathEditor")),
        KeyBinding::new("enter", SubmitPathEdit, Some("PathEditor > Input")),
        KeyBinding::new("down", NextPathSuggestion, Some("PathEditor > Input")),
        KeyBinding::new("up", PreviousPathSuggestion, Some("PathEditor > Input")),
    ]);
}

/// 快捷键的生效范围，决定绑定的按键上下文与冲突判定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ShortcutScope {
    /// 文件区持有焦点时生效；内嵌终端聚焦时不拦截终端自身的按键。
    FileArea,
    /// 文件管理器内任意焦点生效（含内嵌终端），用于开关内嵌终端。
    Everywhere,
    /// 仅内嵌终端聚焦时生效，由终端视图处理。
    Terminal,
}

impl ShortcutScope {
    /// 绑定使用的按键上下文断言。
    fn context(self) -> &'static str {
        match self {
            Self::FileArea => "FileManager && !EmbeddedTerminal",
            Self::Everywhere => "FileManager",
            Self::Terminal => "EmbeddedTerminal",
        }
    }

    /// 两个范围的生效上下文是否相交，用于重复按键的冲突判定。
    fn overlaps(self, other: Self) -> bool {
        !matches!(
            (self, other),
            (Self::FileArea, Self::Terminal) | (Self::Terminal, Self::FileArea)
        )
    }
}

/// 快捷键在偏好设置页所属的功能分组。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ShortcutGroup {
    Tabs,
    Files,
    View,
    Terminal,
    App,
}

impl ShortcutGroup {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Tabs => "标签页与分栏",
            Self::Files => "文件操作",
            Self::View => "视图",
            Self::Terminal => "终端",
            Self::App => "应用",
        }
    }
}

/// 一条可配置快捷键的全部信息。
pub(super) struct ShortcutEntry {
    /// 动作构造器：绑定与解绑各需要一个新实例。
    pub(super) action: fn() -> Box<dyn Action>,
    /// 从配置读取按键，空串表示未绑定。
    pub(super) get: fn(&ShortcutConfig) -> &str,
    /// 写入偏好设置草稿。
    pub(super) get_mut: fn(&mut ShortcutConfig) -> &mut String,
    /// 偏好设置页展示的名称。
    pub(super) label: &'static str,
    /// 偏好设置页的分组。
    pub(super) group: ShortcutGroup,
    pub(super) scope: ShortcutScope,
}

macro_rules! shortcut_entries {
    ($(($action:expr, $field:ident, $group:expr, $label:literal, $scope:expr)),* $(,)?) => {
        /// 全部可配置快捷键，顺序即偏好设置页的展示顺序（按分组排列）。
        pub(super) const SHORTCUTS: &[ShortcutEntry] = &[
            $(
                ShortcutEntry {
                    action: || Box::new($action),
                    get: |config| &config.$field,
                    get_mut: |config| &mut config.$field,
                    label: $label,
                    group: $group,
                    scope: $scope,
                }
            ),*
        ];
    };
}

shortcut_entries! {
    // 标签页与分栏
    (NewTab, new_tab, ShortcutGroup::Tabs, "新建标签页", ShortcutScope::FileArea),
    (CloseTab, close_tab, ShortcutGroup::Tabs, "关闭标签页", ShortcutScope::FileArea),
    (SplitPane, split_pane, ShortcutGroup::Tabs, "向右分栏", ShortcutScope::FileArea),
    (SplitDown, split_down, ShortcutGroup::Tabs, "向下分栏", ShortcutScope::FileArea),
    (SplitLeft, split_left, ShortcutGroup::Tabs, "向左分栏", ShortcutScope::FileArea),
    (SplitUp, split_up, ShortcutGroup::Tabs, "向上分栏", ShortcutScope::FileArea),
    (PaneLeft, pane_left, ShortcutGroup::Tabs, "向左切换活动栏", ShortcutScope::FileArea),
    (PaneRight, pane_right, ShortcutGroup::Tabs, "向右切换活动栏", ShortcutScope::FileArea),
    (PaneUp, pane_up, ShortcutGroup::Tabs, "向上切换活动栏", ShortcutScope::FileArea),
    (PaneDown, pane_down, ShortcutGroup::Tabs, "向下切换活动栏", ShortcutScope::FileArea),
    (ClosePane, close_pane, ShortcutGroup::Tabs, "关闭分栏", ShortcutScope::FileArea),
    // 文件操作
    (FileAction::SelectAll, select_all, ShortcutGroup::Files, "全选", ShortcutScope::FileArea),
    (FileAction::Copy, copy, ShortcutGroup::Files, "复制", ShortcutScope::FileArea),
    (FileAction::Cut, cut, ShortcutGroup::Files, "剪切", ShortcutScope::FileArea),
    (FileAction::Paste, paste, ShortcutGroup::Files, "粘贴", ShortcutScope::FileArea),
    (FileAction::Rename, rename, ShortcutGroup::Files, "重命名", ShortcutScope::FileArea),
    (FileAction::Trash, trash, ShortcutGroup::Files, "移到回收站", ShortcutScope::FileArea),
    (FileAction::NewFolder, new_folder, ShortcutGroup::Files, "新建文件夹", ShortcutScope::FileArea),
    (FileAction::NewDocument, new_document, ShortcutGroup::Files, "新建文档", ShortcutScope::FileArea),
    (FileAction::Properties, properties, ShortcutGroup::Files, "文件夹属性", ShortcutScope::FileArea),
    // 视图
    (ZoomIn, zoom_in, ShortcutGroup::View, "放大文件视图", ShortcutScope::FileArea),
    (ZoomOut, zoom_out, ShortcutGroup::View, "缩小文件视图", ShortcutScope::FileArea),
    (ViewGrid, view_grid, ShortcutGroup::View, "切换到网格视图", ShortcutScope::FileArea),
    (ViewList, view_list, ShortcutGroup::View, "切换到列表视图", ShortcutScope::FileArea),
    // 终端
    (OpenTerminal, open_terminal, ShortcutGroup::Terminal, "打开终端", ShortcutScope::Everywhere),
    (TerminalCopy, terminal_copy, ShortcutGroup::Terminal, "内置终端复制", ShortcutScope::Terminal),
    (TerminalPaste, terminal_paste, ShortcutGroup::Terminal, "内置终端粘贴", ShortcutScope::Terminal),
    // 应用
    (OpenPreferences, open_preferences, ShortcutGroup::App, "偏好设置", ShortcutScope::FileArea),
}

/// 校验整份快捷键配置：逐条解析非空按键，并检查生效范围相交的重复。
pub(super) fn validate(shortcuts: &ShortcutConfig) -> Result<(), String> {
    let mut parsed = vec![(
        ShortcutScope::FileArea,
        parse_shortcut(CANCEL_CUT_KEY, CANCEL_CUT_LABEL)?,
        CANCEL_CUT_LABEL,
    )];
    for entry in SHORTCUTS {
        let value = (entry.get)(shortcuts);
        if value.trim().is_empty() {
            continue;
        }
        let key = parse_shortcut(value, entry.label)?;
        if let Some((_, _, label)) = parsed
            .iter()
            .find(|(scope, other, _)| scope.overlaps(entry.scope) && other == &key)
        {
            return Err(format!("“{label}”和“{}”不能使用相同的快捷键", entry.label));
        }
        parsed.push((entry.scope, key, entry.label));
    }
    Ok(())
}

/// 把一份快捷键配置安装为全局键绑定。整体校验通过后才安装，
/// 任一按键无效即报错且不改动现有绑定。
pub(super) fn bind_all(shortcuts: &ShortcutConfig, cx: &mut App) -> Result<(), String> {
    validate(shortcuts)?;
    let bindings = SHORTCUTS
        .iter()
        .filter_map(|entry| load_binding((entry.get)(shortcuts), (entry.action)(), entry.scope, cx))
        .collect::<Vec<_>>();
    cx.bind_keys(bindings);
    Ok(())
}

/// 解除一份快捷键配置对应的绑定。重新绑定前先移除旧按键，
/// 否则同一动作会同时挂着新旧两个键。
pub(super) fn unbind_all(shortcuts: &ShortcutConfig, cx: &mut App) {
    // 旧配置在加载或保存时已通过校验，这里的解析失败静默跳过即可。
    let unbind = SHORTCUTS
        .iter()
        .filter_map(|entry| {
            let unbind: Box<dyn Action> = Box::new(Unbind((entry.action)().name().into()));
            load_binding((entry.get)(shortcuts), unbind, entry.scope, cx)
        })
        .collect::<Vec<_>>();
    cx.bind_keys(unbind);
}

/// 构造一条键绑定；上下文断言与按键格式都是静态字符串和已校验值，
/// 解析失败返回 None 由调用方决定如何处理。
fn load_binding(
    value: &str,
    action: Box<dyn Action>,
    scope: ShortcutScope,
    cx: &gpui_kit::App,
) -> Option<KeyBinding> {
    if value.trim().is_empty() {
        return None;
    }
    let context = KeyBindingContextPredicate::parse(scope.context()).ok()?;
    KeyBinding::load(value, action, Some(context.into()), false, None, cx).ok()
}

pub(super) fn shortcut_os() -> OperatingSystem {
    if cfg!(target_os = "macos") {
        OperatingSystem::Mac
    } else if cfg!(target_os = "windows") {
        OperatingSystem::Windows
    } else if cfg!(target_os = "freebsd") {
        OperatingSystem::FreeBsd
    } else {
        OperatingSystem::Linux
    }
}

/// 解析单个快捷键。拒绝多键序列与裸字符键（会干扰文本输入），
/// 允许无修饰的功能键（F2、Delete 等）。空串由调用方过滤。
pub(super) fn parse_shortcut(value: &str, label: &str) -> Result<Keystroke, String> {
    let value = value.trim();
    let key = Keystroke::parse_with_os(value, shortcut_os())
        .map_err(|error| format!("{label}的快捷键无效：{error}"))?;
    let modifier = key.modifiers.control || key.modifiers.alt || key.modifiers.platform;
    // 无修饰时只接受功能键：字符键名不会只有一个字符（"f2"、"delete"）。
    let bare_typing_key = key.key.chars().count() == 1;
    if value.split_whitespace().count() != 1
        || key.key.is_empty()
        || key.key_char.is_some()
        || (!modifier && bare_typing_key)
    {
        return Err(format!(
            "{label}需要 Ctrl、Alt 或 Super 组合键，或 F2、Delete 这类功能键"
        ));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::{ShortcutConfig, parse_shortcut, validate};

    #[test]
    fn default_shortcuts_are_valid() {
        validate(&ShortcutConfig::default()).expect("默认键位必须能整体通过校验");
    }

    #[test]
    fn duplicate_keys_conflict_only_when_scopes_overlap() {
        // 默认配置里文件区复制与终端复制本就同为 ctrl-c：范围不相交，合法。
        let mut config = ShortcutConfig::default();
        assert_eq!(config.copy, config.terminal_copy);
        assert!(validate(&config).is_ok());

        // “打开终端”在终端聚焦时也生效，与终端复制冲突。
        config.open_terminal = config.terminal_copy.clone();
        assert!(validate(&config).is_err());

        // 同范围重复必然冲突。
        let mut config = ShortcutConfig::default();
        config.trash = config.rename.clone();
        assert_eq!(
            validate(&config).unwrap_err(),
            "“重命名”和“移到回收站”不能使用相同的快捷键"
        );

        let mut config = ShortcutConfig {
            terminal_copy: "escape".into(),
            ..ShortcutConfig::default()
        };
        assert!(validate(&config).is_ok());
        config.trash = "escape".into();
        assert_eq!(
            validate(&config).unwrap_err(),
            "“取消剪切”和“移到回收站”不能使用相同的快捷键"
        );
    }

    #[test]
    fn surrounding_whitespace_does_not_hide_conflicts() {
        let config = ShortcutConfig {
            trash: "  ctrl-c  ".into(),
            ..ShortcutConfig::default()
        };
        assert_eq!(
            validate(&config).unwrap_err(),
            "“复制”和“移到回收站”不能使用相同的快捷键"
        );
    }

    #[test]
    fn function_keys_are_valid_without_modifiers() {
        assert!(parse_shortcut("f2", "重命名").is_ok());
        assert!(parse_shortcut("delete", "移到回收站").is_ok());
        assert!(parse_shortcut("ctrl-shift-c", "测试").is_ok());
        // 裸字符键会吞掉文本输入，必须拒绝。
        assert!(parse_shortcut("a", "测试").is_err());
        assert!(parse_shortcut("ctrl-`", "测试").is_ok());
    }
}
