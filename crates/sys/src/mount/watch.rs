use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

use futures_lite::io::{AsyncBufReadExt as _, BufReader};
use rustix::process::{Signal, getpid, getppid, set_parent_process_death_signal};

pub async fn watch_volumes(mut changed: impl FnMut()) -> io::Result<()> {
    let mut command = Command::new("gio");
    command
        .args(["mount", "--monitor"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let parent = getpid();
    // 子进程在应用异常退出时也要停止；这里只调用 Linux 系统调用。
    unsafe {
        command.pre_exec(move || {
            set_parent_process_death_signal(Some(Signal::TERM))?;
            if getppid() != Some(parent) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            Ok(())
        });
    }
    let mut child = async_process::Command::from(command)
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child.stdout.take().expect("已为卷监视器配置标准输出");
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line).await? != 0 {
        // 第一行表示监听已就绪，也刷新一次以补上启动扫描期间可能漏掉的事件。
        changed();
        line.clear();
    }
    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "卷监视器已退出",
    ))
}
