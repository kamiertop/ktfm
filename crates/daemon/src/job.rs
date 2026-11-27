//! 文件任务和逐项执行结果。

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use crate::files;
use crate::files::{CreateKind, TransferMode};

/// 可交给后台工作线程执行的文件操作。
pub enum FileJob {
    Create {
        path: PathBuf,
        kind: CreateKind,
    },
    Rename {
        source: PathBuf,
        target: PathBuf,
    },
    Transfer {
        sources: Arc<[PathBuf]>,
        directory: PathBuf,
        mode: TransferMode,
    },
    Trash(Vec<PathBuf>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// 供界面显示进度和处理完成结果的任务类别。
pub enum JobKind {
    Create,
    Rename,
    Transfer { mode: TransferMode },
    Trash,
}

/// 单个项目的路径及执行结果。批量任务允许部分成功。
pub struct ItemResult {
    pub source: Option<PathBuf>,
    pub target: Option<PathBuf>,
    pub result: io::Result<()>,
}

/// 文件任务的逐项结果，按输入顺序排列。
pub struct JobResult {
    pub kind: JobKind,
    pub items: Vec<ItemResult>,
}

impl FileJob {
    /// 执行前可读取此类别以显示进度。
    #[must_use]
    pub fn kind(&self) -> JobKind {
        match self {
            Self::Create { .. } => JobKind::Create,
            Self::Rename { .. } => JobKind::Rename,
            Self::Transfer { mode, .. } => JobKind::Transfer { mode: *mode },
            Self::Trash(_) => JobKind::Trash,
        }
    }

    /// 同步执行任务，并继续处理部分失败的批量任务。
    #[must_use = "应检查逐项结果，文件任务可能部分失败"]
    pub fn run(self) -> JobResult {
        let kind = self.kind();
        let items = match self {
            Self::Create { path, kind } => vec![ItemResult {
                result: files::create(&path, kind),
                source: None,
                target: Some(path),
            }],
            Self::Rename { source, target } => vec![ItemResult {
                result: files::rename(&source, &target),
                source: Some(source),
                target: Some(target),
            }],
            Self::Transfer {
                sources,
                directory,
                mode,
            } => sources
                .iter()
                .map(|source| ItemResult {
                    result: files::transfer(source, &directory, mode),
                    source: Some(source.clone()),
                    target: source.file_name().map(|name| directory.join(name)),
                })
                .collect(),
            Self::Trash(paths) => paths
                .into_iter()
                .map(|path| ItemResult {
                    result: sys::trash::put(&path),
                    source: Some(path),
                    target: None,
                })
                .collect(),
        };
        JobResult { kind, items }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn cut_reports_each_source_and_preserves_existing_targets() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let directory = root.path().join("destination");
        std::fs::create_dir(&directory)?;
        let moved = root.path().join("moved");
        let blocked = root.path().join("blocked");
        let existing = directory.join("blocked");
        std::fs::write(&moved, "move me")?;
        std::fs::write(&blocked, "keep me")?;
        std::fs::write(&existing, "keep the destination too")?;

        let outcome = FileJob::Transfer {
            sources: [blocked.clone(), moved.clone()].into(),
            directory: directory.clone(),
            mode: TransferMode::Move,
        }
        .run();

        let [failure, success] = outcome.items.as_slice() else {
            panic!("每个源路径都应该有执行结果");
        };
        assert_eq!(failure.source.as_ref(), Some(&blocked));
        assert_eq!(failure.target.as_ref(), Some(&existing));
        assert_eq!(
            failure.result.as_ref().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(success.source.as_ref(), Some(&moved));
        assert!(success.result.is_ok());
        assert!(!moved.exists());
        assert_eq!(std::fs::read_to_string(directory.join("moved"))?, "move me");
        assert_eq!(std::fs::read_to_string(blocked)?, "keep me");
        assert_eq!(
            std::fs::read_to_string(existing)?,
            "keep the destination too"
        );
        Ok(())
    }
}
