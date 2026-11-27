//! 在后台执行本地文件操作，目标同名时保留原有文件。

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::{fs, io};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// 用于创建空文档或空目录的文件任务。
pub enum CreateKind {
    Document,
    Directory,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
/// 决定传输成功后是否保留源文件。
pub enum TransferMode {
    #[default]
    Copy,
    Move,
}

/// 将单个文件名拼接到父目录。拒绝空名称、路径分隔符和特殊路径组件。
pub fn named_path(parent: &Path, name: &str) -> io::Result<PathBuf> {
    let mut components = Path::new(name).components();
    if name.contains(['/', '\0'])
        || name.trim().is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "请输入有效的文件名",
        ));
    }
    Ok(parent.join(name))
}

/// 创建空文件或空目录。如果目标已存在，返回 `AlreadyExists`。
pub fn create(path: &Path, kind: CreateKind) -> io::Result<()> {
    match kind {
        CreateKind::Document => fs::File::create_new(path).map(drop),
        CreateKind::Directory => fs::create_dir(path),
    }
}

/// 重命名且拒绝覆盖已有目标。源路径与目标路径相同时直接成功。
pub fn rename(source: &Path, target: &Path) -> io::Result<()> {
    if source == target {
        return Ok(());
    }
    rename_without_replace(source, target)
}

#[cfg(target_os = "linux")]
fn rename_without_replace(source: &Path, target: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, source, CWD, target, RenameFlags::NOREPLACE).map_err(Into::into)
}

#[cfg(not(target_os = "linux"))]
fn rename_without_replace(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// 将源项目传输到指定目录，保留原有的同名文件。移动时保留符号链接本身。
pub fn transfer(source: &Path, directory: &Path, mode: TransferMode) -> io::Result<()> {
    let name = source.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let directory = directory.canonicalize()?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_dir() && directory.starts_with(source.canonicalize()?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "不能将目录放入自身或子目录",
        ));
    }
    let target = directory.join(name);
    if mode == TransferMode::Move {
        if source
            .parent()
            .map(Path::canonicalize)
            .transpose()?
            .as_deref()
            == Some(&directory)
        {
            return Ok(());
        }
        return match rename_without_replace(source, &target) {
            // 由系统工具处理跨设备移动，并在并发出现同名项时拒绝覆盖。
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => run(Command::new("mv")
                .args(["-T", "--update=none-fail", "--"])
                .arg(source)
                .arg(target)),
            result => result,
        };
    }
    match target.symlink_metadata() {
        Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    // 复制到临时目录，成功后再发布，失败不会留下半份目标文件。
    let staging = tempfile::Builder::new()
        .prefix(".ktfm-copy-")
        .tempdir_in(directory)?;
    let staged = staging.path().join(name);
    run(Command::new("cp")
        .args(["-a", "-T", "--"])
        .arg(source)
        .arg(&staged))?;
    rename_without_replace(&staged, &target)
}

fn run(command: &mut Command) -> io::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(format!(
            "{}：{}",
            output.status,
            error.trim()
        )))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn creates_empty_documents_without_overwriting_existing_entries() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("document.txt");
        create(&path, CreateKind::Document)?;
        assert_eq!(fs::metadata(&path)?.len(), 0);
        fs::write(&path, b"original")?;
        assert_eq!(
            create(&path, CreateKind::Document).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&path)?, b"original");

        let link = root.path().join("link");
        let missing = root.path().join("missing");
        std::os::unix::fs::symlink(&missing, &link)?;
        assert_eq!(
            create(&link, CreateKind::Document).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(!missing.exists());
        Ok(())
    }

    #[test]
    fn preserves_conflicts_and_rejects_recursive_copy() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir_all(source.join("child"))?;
        fs::create_dir(&destination)?;
        fs::write(source.join("file"), b"original")?;
        assert!(named_path(root.path(), "../other").is_err());
        assert!(named_path(root.path(), "source/").is_err());
        assert!(transfer(&source, &source.join("child"), TransferMode::Copy).is_err());
        transfer(&source, &destination, TransferMode::Copy)?;
        assert_eq!(fs::read(destination.join("source/file"))?, b"original");
        assert_eq!(
            rename(&source, &destination).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        Ok(())
    }

    #[test]
    fn moves_symlinks_without_following_them() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("link");
        let dest = root.path().join("dest");
        fs::create_dir(&dest)?;
        std::os::unix::fs::symlink("missing", &source)?;
        transfer(&source, &dest, TransferMode::Move)?;
        assert!(source.symlink_metadata().is_err());
        assert_eq!(fs::read_link(dest.join("link"))?, Path::new("missing"));
        Ok(())
    }
}
