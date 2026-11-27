use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

use futures_lite::io::{AsyncBufReadExt as _, AsyncRead, BufReader};
use rustix::process::{self, Signal};

/// 监听 GIO 卷事件。监视进程就绪时也调用一次回调，以补读启动期间的变化。
pub async fn watch_volumes(changed: impl FnMut()) -> io::Result<()> {
    let mut command = Command::new("gio");
    command
        .args(["mount", "--monitor"])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let parent = process::getpid();
    // 父进程死亡信号必须在子进程中、执行 GIO 前设置，Command 没有对应的安全接口。
    // SAFETY: fork 后只调用 Linux 的 prctl 和 getppid，捕获值为可复制的 PID，
    // 错误路径只转换操作系统错误码或 ErrorKind，不分配内存或获取锁。
    unsafe {
        command.pre_exec(move || {
            process::set_parent_process_death_signal(Some(Signal::TERM))?;
            if process::getppid() != Some(parent) {
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
