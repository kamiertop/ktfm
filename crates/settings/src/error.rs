use std::io;
use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("标签数据库错误: {0}")]
    TagDatabase(#[from] turso::Error),

    #[error("标签数据库数据无法解析: {0}")]
    InvalidTagData(String),

    #[error("标签路径编码错误: {0}")]
    TagPathEncoding(#[from] serde_json::Error),

    #[error("无法确定用户配置目录")]
    NoConfigDir,

    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),

    #[error("配置解析失败: {0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("配置序列化失败: {0}")]
    ConfigSerialize(#[from] toml::ser::Error),
}
