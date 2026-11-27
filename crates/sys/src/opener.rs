use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
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

/// 把一行启动命令拆成（程序, 参数）。规则对齐 POSIX shell 的常见子集：
/// 单引号内一切字符原样；双引号内 `\"`、`\\` 转义；引号外反斜杠转义
/// 下一字符；其余按空白分割。空行返回 None。
/// 不引第三方 shell 解析库：只服务"程序 + 少量参数"的启动场景。
#[must_use]
pub fn split_command(line: &str) -> Option<(String, Vec<String>)> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in line.chars() {
        if in_single {
            // 单引号内无任何转义，这是 POSIX 语义，也是最安全的引用方式。
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
            continue;
        }
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' => in_single = true,
            '"' => in_double = !in_double,
            // 双引号内的空白属于参数本身，不分割。
            c if c.is_whitespace() && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    // 未闭合引号也按已收集内容处理：启动命令容错优先于严格解析。
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut tokens = tokens.into_iter();
    let program = tokens.next()?;
    Some((program, tokens.collect()))
}

/// 已知终端的"起始目录"参数映射。返回 None 的终端依赖 current_dir
/// 继承工作目录（gnome-terminal/kgx/xdg-terminal-exec 及未知程序）。
/// 需要显式参数的原因：复用已有进程的终端（如 ghostty --single-instance）
/// 收不到 current_dir，只能靠命令行参数传目录。
fn known_directory_args(program_name: &str, directory: &Path) -> Option<Vec<OsString>> {
    let joined = |flags: &[&str]| {
        flags
            .iter()
            .map(OsString::from)
            .chain([directory.as_os_str().to_os_string()])
            .collect()
    };
    match program_name {
        "kitty" => Some(joined(&["--directory"])),
        "alacritty" | "foot" => Some(joined(&["--working-directory"])),
        "wezterm" => Some(joined(&["start", "--cwd"])),
        "konsole" => Some(joined(&["--workdir"])),
        "ghostty" => {
            // ghostty 用 = 连接值的形式。
            let mut argument = OsString::from("--working-directory=");
            argument.push(directory);
            Some(vec![argument])
        }
        _ => None,
    }
}

/// 在指定目录启动外部终端。program 支持带参数（由 split_command 拆分）；
/// 留空则按候选列表自动探测。
pub fn open_terminal(directory: &Path, program: &str) -> io::Result<std::process::Child> {
    use std::process::Stdio;

    let launch = |program: &str| {
        let Some((name, args)) = split_command(program) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "外部终端命令为空",
            ));
        };
        let mut command = Command::new(&name);
        command.args(&args);
        let extra = Path::new(&name)
            .file_name()
            .and_then(|file| file.to_str())
            .and_then(|file| known_directory_args(file, directory));
        // 用户自带参数在前、目录参数在后：已知终端的目录选项重复时
        // 以显式追加为准（最后一个生效是常见 CLI 约定）。
        command.args(extra.unwrap_or_default());
        command
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    if !program.trim().is_empty() {
        return launch(program);
    }
    for candidate in [
        "xdg-terminal-exec",
        "ghostty",
        "kgx",
        "gnome-terminal",
        "konsole",
        "x-terminal-emulator",
        "xterm",
    ] {
        match launch(candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => return result,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "未找到外部终端，请在设置中指定程序",
    ))
}

/// 校验外部终端程序可执行：含路径分隔符按字面路径检查，否则逐个搜索
/// $PATH。返回解析到的完整路径。
#[must_use]
pub fn resolve_executable(token: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    fn is_executable(path: &Path) -> bool {
        std::fs::metadata(path)
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    }

    if token.contains('/') {
        return is_executable(Path::new(token)).then(|| PathBuf::from(token));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(token))
        .find(|full| is_executable(full))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_command_handles_quotes_and_escapes() {
        assert_eq!(split_command("ghostty"), Some(("ghostty".into(), vec![])));
        assert_eq!(
            split_command("kitty --directory '/a b'"),
            Some(("kitty".into(), vec!["--directory".into(), "/a b".into()]))
        );
        assert_eq!(
            split_command(r#"wezterm start --cwd "/tmp/x y""#),
            Some((
                "wezterm".into(),
                vec!["start".into(), "--cwd".into(), "/tmp/x y".into()]
            ))
        );
        // 引号外的反斜杠转义，双引号内 \" 保留字面引号。
        assert_eq!(
            split_command(r#"alacritty --working-directory /home/a\ b"#),
            Some((
                "alacritty".into(),
                vec!["--working-directory".into(), "/home/a b".into(),]
            ))
        );
        assert_eq!(split_command("   "), None);
        // 多余空白归一，不产生空 token。
        assert_eq!(
            split_command("  foot   --working-directory  "),
            Some(("foot".into(), vec!["--working-directory".into()]))
        );
    }

    #[test]
    fn known_directory_args_maps_supported_terminals() {
        let dir = Path::new("/tmp");
        assert_eq!(
            known_directory_args("kitty", dir),
            Some(vec!["--directory".into(), "/tmp".into()])
        );
        // 绝对路径按 file_name 匹配是调用方职责，这里只验证名字直配。
        assert_eq!(known_directory_args("gnome-terminal", dir), None);
        assert_eq!(known_directory_args("unknown-term", dir), None);
        assert_eq!(
            known_directory_args("ghostty", dir),
            Some(vec![OsString::from("--working-directory=/tmp")])
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_executable_finds_path_and_literal() {
        assert!(resolve_executable("sh").is_some());
        assert_eq!(
            resolve_executable("/bin/sh"),
            Some(PathBuf::from("/bin/sh"))
        );
        assert!(resolve_executable("/definitely/not/here").is_none());
        assert!(resolve_executable("ktfm-no-such-tool").is_none());
    }
}
