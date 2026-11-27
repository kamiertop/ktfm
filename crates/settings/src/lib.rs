//! 应用配置、工作区和标签的本地持久化。

pub mod command;
pub mod config;
pub mod error;
mod file_lock;
pub mod tag;
pub mod tag_store;

pub use error::{Error, Result};
