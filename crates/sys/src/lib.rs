//! 应用与桌面环境、系统工具和存储设备的集成。

pub mod media;
pub mod mime;
pub mod mount;
pub mod opener;
#[cfg(target_os = "linux")]
pub mod thumbnail_cache;
pub mod trash;
