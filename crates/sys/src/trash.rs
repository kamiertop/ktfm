use std::io;
use std::path::Path;
use std::process::Command;

/// 交给桌面回收站处理。失败时保留原文件。
pub fn put(path: &Path) -> io::Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(io::ErrorKind::Unsupported.into());
    }
    let output = Command::new("gio")
        .args(["trash", "--"])
        .arg(path)
        .output()?;
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
