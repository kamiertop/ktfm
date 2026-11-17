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
actions!(browser, [NewTab, CloseTab, OpenTerminal, OpenPreferences]);
// 内嵌终端聚焦时由终端视图处理的剪贴板动作。
actions!(browser, [TerminalCopy, TerminalPaste]);

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
    pub(super) scope: ShortcutScope,
}

macro_rules! shortcut_entries {
    ($(($action:expr, $field:ident, $label:literal, $scope:expr)),* $(,)?) => {
        /// 全部可配置快捷键，顺序即偏好设置页的展示顺序。
        pub(super) const SHORTCUTS: &[ShortcutEntry] = &[
            $(
                ShortcutEntry {
                    action: || Box::new($action),
                    get: |config| &config.$field,
                    get_mut: |config| &mut config.$field,
                    label: $label,
                    scope: $scope,
                }
            ),*
        ];
    };
}

shortcut_entries! {
    (NewTab, new_tab, "新建标签页", ShortcutScope::FileArea),
    (CloseTab, close_tab, "关闭标签页", ShortcutScope::FileArea),
    (OpenTerminal, open_terminal, "打开终端", ShortcutScope::Everywhere),
    (OpenPreferences, open_preferences, "偏好设置", ShortcutScope::FileArea),
    (FileAction::SelectAll, select_all, "全选", ShortcutScope::FileArea),
    (FileAction::Copy, copy, "复制", ShortcutScope::FileArea),
    (FileAction::Cut, cut, "剪切", ShortcutScope::FileArea),
    (FileAction::Paste, paste, "粘贴", ShortcutScope::FileArea),
    (FileAction::Rename, rename, "重命名", ShortcutScope::FileArea),
    (FileAction::Trash, trash, "移到回收站", ShortcutScope::FileArea),
    (FileAction::NewFolder, new_folder, "新建文件夹", ShortcutScope::FileArea),
    (FileAction::NewDocument, new_document, "新建文档", ShortcutScope::FileArea),
    (FileAction::Properties, properties, "文件夹属性", ShortcutScope::FileArea),
    (TerminalCopy, terminal_copy, "内置终端复制", ShortcutScope::Terminal),
    (TerminalPaste, terminal_paste, "内置终端粘贴", ShortcutScope::Terminal),
}

/// 校验整份快捷键配置：逐条解析非空按键，并检查生效范围相交的重复。
pub(super) fn validate(shortcuts: &ShortcutConfig) -> Result<(), String> {
    let mut parsed: Vec<(ShortcutScope, Keystroke, &str)> = Vec::new();
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
            return Err(format!(
                "“{label}”和“{}”不能使用相同的快捷键",
                entry.label
            ));
        }
        parsed.push((entry.scope, key, entry.label));
    }
    Ok(())
}

/// 把一份快捷键配置安装为全局键绑定。整体校验通过后才安装，
/// 任一按键无效即报错且不改动现有绑定。
pub(super) fn bind_all(
    shortcuts: &ShortcutConfig,
    cx: &mut gpui_kit::App,
) -> Result<(), String> {
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
pub(super) fn unbind_all(shortcuts: &ShortcutConfig, cx: &mut gpui_kit::App) {
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
        assert!(validate(&config).is_err());
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
