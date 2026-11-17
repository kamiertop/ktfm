//! 终端配置。

use serde::{Deserialize, Serialize};

pub const DEFAULT_BACKGROUND: &str = "#161A22";
pub const DEFAULT_FONT_SIZE: f32 = 13.;
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<f32> = 9.0..=28.0;

pub const DEFAULT_COPY_SHORTCUT: &str = "ctrl-c";
pub const DEFAULT_PASTE_SHORTCUT: &str = "ctrl-v";

/// 终端配置根。
///
/// f32 不实现 Eq（NaN 无全序语义），含它的配置结构只能 derive(PartialEq)，
/// 对"比较配置是否变化"的场景已经足够。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// 快捷键打开的终端类型。
    pub mode: TerminalMode,
    /// 外部终端启动命令：程序名或绝对路径，可带参数
    /// （如 "kitty --single-instance"）。字符串而非 PathBuf 以支持参数；
    /// 旧的纯路径值在 TOML 里本就是字符串，序列化格式双向兼容。
    /// 留空表示按候选列表自动探测。
    pub external_program: String,
    /// 内嵌终端字号（像素）。
    pub font_size: f32,
    /// 内嵌终端默认背景色，格式为 #RRGGBB。
    pub background: String,
    /// 会话未指定光标样式时使用的形状。
    pub cursor_shape: TerminalCursorShape,
    /// 内嵌终端选区复制快捷键。
    pub copy_shortcut: String,
    /// 内嵌终端剪贴板粘贴快捷键。
    pub paste_shortcut: String,
}

// 默认值非类型自然默认（字号 13px、程序为空串走自动探测），
// 按仓库约定手写 Default。
impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            mode: TerminalMode::default(),
            external_program: String::new(),
            font_size: DEFAULT_FONT_SIZE,
            background: DEFAULT_BACKGROUND.into(),
            cursor_shape: TerminalCursorShape::default(),
            copy_shortcut: DEFAULT_COPY_SHORTCUT.into(),
            paste_shortcut: DEFAULT_PASTE_SHORTCUT.into(),
        }
    }
}

/// 快捷键打开的终端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMode {
    /// 内嵌终端。
    #[default]
    Embedded,
    /// 外部终端。
    External,
}

/// 内嵌终端的默认光标形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCursorShape {
    #[default]
    Block,
    Beam,
    Underline,
}
