use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

use futures_lite::io::{AsyncBufReadExt as _, AsyncRead, BufReader};
use rustix::process::{Signal, getpid, getppid, set_parent_process_death_signal};

pub async fn watch_volumes(changed: impl FnMut()) -> io::Result<()> {
    let mut command = Command::new("gio");
    command
        .args(["mount", "--monitor"])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    let stderr = child.stderr.take().expect("已为卷监视器配置标准错误");
    watch_output(stdout, stderr, changed).await
}

async fn watch_output(
    stdout: impl AsyncRead + Unpin,
    stderr: impl AsyncRead + Unpin,
    mut changed: impl FnMut(),
) -> io::Result<()> {
    futures_lite::future::or(
        async {
            // 即使个别 D-Bus 后端初始化失败，GIO 也可能继续运行并输出就绪提示。
            let mut line = Vec::new();
            if BufReader::new(stderr).read_until(b'\n', &mut line).await? != 0 {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&line).trim().to_owned(),
                ));
            }
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "卷监视器已退出",
            ))
        },
        async {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            while reader.read_until(b'\n', &mut line).await? != 0 {
                // 就绪时也刷新一次，补上启动扫描期间可能漏掉的事件。
                changed();
                line.clear();
            }
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "卷监视器已退出",
            ))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_failed_backend_even_if_gio_prints_its_ready_banner() {
        let mut changed = false;
        let result = futures_lite::future::block_on(watch_output(
            b"Monitoring events. Press Ctrl+C to quit.\n".as_slice(),
            b"Error creating proxy: Could not connect\n".as_slice(),
            || changed = true,
        ));
        assert!(!changed);
        assert_eq!(
            result.unwrap_err().to_string(),
            "Error creating proxy: Could not connect"
        );
    }
}
