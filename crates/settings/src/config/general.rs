use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct GeneralConfig {
    pub activation_policy: ActivationPolicy,
    pub startup_behavior: StartupBehavior,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivationPolicy {
    /// 双击进去
    #[default]
    Double,
    /// 单击进入
    Single,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartupBehavior {
    /// 启动时打开上次的目录
    #[default]
    RestoreLastDir,
    /// 启动时打开指定的目录
    OpenSpecifiedDir(PathBuf),
    /// 启动时打开用户的主目录
    HomeDir,
}