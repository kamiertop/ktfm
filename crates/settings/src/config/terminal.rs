//! 终端配置。
//!
//! 当前应用只实现外部终端；`mode` 与 `font_size` 是为将来的内嵌终端
//! 预留的字段——旧配置文件里可能已有这两个值，重新实现时直接复用，
//! 不要从 schema 中删除。

use serde::{Deserialize, Serialize};

/// 终端配置根。
///
/// f32 不实现 Eq（NaN 无全序语义），含它的配置结构只能 derive(PartialEq)，
/// 对"比较配置是否变化"的场景已经足够。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// 快捷键打开的终端类型。当前无论取值都走外部终端。
    pub mode: TerminalMode,
    /// 外部终端启动命令：程序名或绝对路径，可带参数
    /// （如 "kitty --single-instance"）。字符串而非 PathBuf 以支持参数；
    /// 旧的纯路径值在 TOML 里本就是字符串，序列化格式双向兼容。
    /// 留空表示按候选列表自动探测。
    pub external_program: String,
    /// 内嵌终端字号（像素）。预留给将来的实现，当前无消费者。
    pub font_size: f32,
}

// 默认值非类型自然默认（字号 13px、程序为空串走自动探测），
// 按仓库约定手写 Default。
impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            mode: TerminalMode::default(),
            external_program: String::new(),
            font_size: 13.0,
        }
    }
}

/// 快捷键打开的终端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMode {
    /// 内嵌终端（当前未实现，配置值保留）。
    #[default]
    Embedded,
    /// 外部终端。
    External,
}
