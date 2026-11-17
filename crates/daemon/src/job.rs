//! 文件任务和逐项执行结果。

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use crate::files;

pub enum FileJob {
    Create {
        path: PathBuf,
        folder: bool,
    },
    Rename {
        source: PathBuf,
        target: PathBuf,
    },
    Transfer {
        sources: Arc<[PathBuf]>,
        directory: PathBuf,
        cut: bool,
    },
    Trash(Vec<PathBuf>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Create,
    Rename,
    Transfer { cut: bool },
    Trash,
}

pub struct ItemResult {
    pub source: Option<PathBuf>,
    pub target: Option<PathBuf>,
    pub result: io::Result<()>,
}

pub struct JobResult {
    pub kind: JobKind,
    pub items: Vec<ItemResult>,
}

impl FileJob {
    pub fn kind(&self) -> JobKind {
        match self {
            Self::Create { .. } => JobKind::Create,
            Self::Rename { .. } => JobKind::Rename,
            Self::Transfer { cut, .. } => JobKind::Transfer { cut: *cut },
            Self::Trash(_) => JobKind::Trash,
        }
    }

    pub fn run(self) -> JobResult {
        let kind = self.kind();
        let items = match self {
            Self::Create { path, folder } => vec![ItemResult {
                result: files::create(&path, folder),
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
                cut,
            } => sources
                .iter()
                .map(|source| ItemResult {
                    result: files::transfer(source, &directory, cut),
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
            cut: true,
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
