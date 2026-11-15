use std::io;
use std::path::Path;
use std::process::Command;

/// 使用系统默认应用打开文件。调用方应在后台执行。
pub fn open(path: &Path) -> io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(unix) {
        Command::new("xdg-open")
    } else {
        return Err(io::Error::from(io::ErrorKind::Unsupported));
    };

    let status = command.arg(path).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("系统打开程序退出：{status}")))
    }
}
