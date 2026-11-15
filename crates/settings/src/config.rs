pub mod action;
pub mod general;
pub mod operations;
pub mod star;
pub mod tab;
pub mod view;
pub mod workspace;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{fs, io};

pub use action::ActionConfig;
pub use general::GeneralConfig;
pub use operations::OperationsConfig;
pub use view::ViewConfig;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
// 缺字段时用默认值补齐，兼容旧版本配置和空文件
#[serde(default)]
pub struct AppConfig {
    pub actions: ActionConfig,
    pub general: GeneralConfig,
    pub view: ViewConfig,
    pub operations: OperationsConfig,
}

pub(crate) const APP_NAME: &str = "ktfm";
const CONFIG_FILE_NAME: &str = "config.toml";

impl AppConfig {
    /// 加载默认配置文件
    pub fn load_from_file() -> Result<Self> {
        let project_dirs = ProjectDirs::from("", "", APP_NAME).ok_or(Error::NoConfigDir)?;
        Self::load_from(project_dirs.config_dir())
    }

    /// 从指定目录加载配置，文件不存在时先落默认配置
    fn load_from(config_dir: &Path) -> Result<Self> {
        let config_file = config_dir.join(CONFIG_FILE_NAME);
        let toml_str = match fs::read_to_string(&config_file) {
            Ok(content) => content,
            // 文件不存在：先写入默认配置再读回
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Self::write_default_config_file(config_dir)?;
                fs::read_to_string(&config_file)?
            }
            Err(e) => return Err(e.into()),
        };

        Ok(toml::from_str(&toml_str)?)
    }

    /// 写入默认配置文件（仅在文件不存在时创建，绝不覆盖已有文件）
    fn write_default_config_file(config_dir: &Path) -> Result<()> {
        fs::create_dir_all(config_dir)?;
        let config_file = config_dir.join(CONFIG_FILE_NAME);
        let toml_str = toml::to_string_pretty(&Self::default())?;

        // 使用硬链接发布完整文件，其他实例已创建配置时保留现有文件。
        Self::write_config(&config_file, &toml_str, |tmp, target| {
            match fs::hard_link(tmp, target) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                result => result,
            }
        })
    }

    pub fn save_to_file(&self) -> Result<()> {
        let project_dirs = ProjectDirs::from("", "", APP_NAME).ok_or(Error::NoConfigDir)?;
        self.save_to(project_dirs.config_dir())
    }

    /// 保存配置到指定目录
    fn save_to(&self, config_dir: &Path) -> Result<()> {
        fs::create_dir_all(config_dir)?;
        let config_file = config_dir.join(CONFIG_FILE_NAME);
        let toml_str = toml::to_string_pretty(self)?;

        Self::atomic_write(&config_file, &toml_str)
    }

    /// 原子写入：先写临时文件，再 rename 替换目标
    fn atomic_write(path: &Path, content: &str) -> Result<()> {
        Self::write_config(path, content, |tmp, target| fs::rename(tmp, target))
    }

    fn write_config(
        path: &Path,
        content: &str,
        publish: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> Result<()> {
        let tmp_path = Self::tmp_path_for(path);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(&tmp_path)?;
        let result = (|| {
            file.write_all(content.as_bytes())?;
            Self::inherit_mode(path, &tmp_path)?;
            file.sync_all()?;
            drop(file);
            publish(&tmp_path, path)?;
            Ok(())
        })();
        let _ = fs::remove_file(&tmp_path);

        #[cfg(unix)]
        if result.is_ok()
            && let Some(parent) = path.parent()
            && let Ok(dir) = fs::File::open(parent)
        {
            let _ = dir.sync_all();
        }

        result
    }

    /// 对齐 tmp 权限：目标已存在则沿用其权限，新建文件则为 0600
    #[cfg(unix)]
    fn inherit_mode(target: &Path, tmp: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mode = match fs::metadata(target) {
            // 掩掉文件类型位，只保留权限位
            Ok(m) => m.permissions().mode() & 0o777,
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0o600,
            Err(e) => return Err(e.into()),
        };
        fs::set_permissions(tmp, fs::Permissions::from_mode(mode))?;
        Ok(())
    }

    /// 非 Unix 平台没有 mode 位，空实现
    #[cfg(not(unix))]
    fn inherit_mode(_target: &Path, _tmp: &Path) -> Result<()> {
        Ok(())
    }

    /// 生成同目录临时文件路径，名字掺 PID/线程/时间戳避免并发冲突
    fn tmp_path_for(path: &Path) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        let pid = std::process::id();
        let thread_id = format!("{:?}", std::thread::current().id());
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        path.with_file_name(format!("{file_name}.{pid}_{thread_id}_{nanos}.tmp"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// 每个测试独享的干净临时目录
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ktfm-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_existing_mode() {
        let dir = temp_dir("atomic1");
        let file = dir.join(CONFIG_FILE_NAME);

        // 先落一份"用户 chmod 成 0640"的文件，再原子覆写
        fs::write(&file, "a = 1").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        AppConfig::atomic_write(&file, "a = 2").unwrap();

        assert_eq!(mode_of(&file), 0o640, "已存在的文件应沿用用户设置的权限");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_defaults_new_file_to_0600() {
        let dir = temp_dir("atomic2");
        let file = dir.join(CONFIG_FILE_NAME);

        AppConfig::atomic_write(&file, "a = 1").unwrap();

        assert_eq!(mode_of(&file), 0o600, "新建文件应默认仅所有者可读写");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn inherit_mode_follows_existing_target() {
        let dir = temp_dir("inherit1");
        let target = dir.join(CONFIG_FILE_NAME);
        let tmp = dir.join("some.tmp");

        fs::write(&target, "").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&tmp, "").unwrap();
        AppConfig::inherit_mode(&target, &tmp).unwrap();

        assert_eq!(mode_of(&tmp), 0o640, "tmp 应沿用目标已有权限");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn inherit_mode_defaults_to_0600_when_target_missing() {
        let dir = temp_dir("inherit2");
        let tmp = dir.join("some.tmp");

        fs::write(&tmp, "").unwrap();
        AppConfig::inherit_mode(&dir.join(CONFIG_FILE_NAME), &tmp).unwrap();

        assert_eq!(mode_of(&tmp), 0o600, "目标不存在时 tmp 应为 0600");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tmp_path_is_same_dir_and_unique() {
        let target = Path::new("/home/u/.config/ktfm/config.toml");
        let a = AppConfig::tmp_path_for(target);
        let b = AppConfig::tmp_path_for(target);

        assert_eq!(
            a.parent(),
            target.parent(),
            "tmp 必须和目标同目录（rename 原子性前提）"
        );
        assert!(a.to_string_lossy().ends_with(".tmp"));
        assert_ne!(a, b, "两次生成的路径应不同，避免并发冲突");
    }

    #[test]
    fn write_default_creates_parsable_config() {
        let dir = temp_dir("default1");

        AppConfig::write_default_config_file(&dir).unwrap();

        let content = fs::read_to_string(dir.join(CONFIG_FILE_NAME)).unwrap();
        toml::from_str::<AppConfig>(&content).expect("默认配置内容必须是合法 TOML 且能解析");
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "不应残留临时文件");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_default_never_overwrites_existing() {
        let dir = temp_dir("default2");
        let file = dir.join(CONFIG_FILE_NAME);

        fs::write(&file, "# user custom config\n").unwrap();
        AppConfig::write_default_config_file(&dir).unwrap();

        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content, "# user custom config\n", "已有配置不应被覆盖");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_cleans_up_after_failed_rename() {
        let dir = temp_dir("atomic-failure");
        let target = dir.join(CONFIG_FILE_NAME);
        fs::create_dir(&target).unwrap();

        assert!(AppConfig::atomic_write(&target, "a = 1").is_err());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_accepts_legacy_zoom_action() {
        use action::CtrlWheelAction;

        let config: AppConfig = toml::from_str("[actions]\nctrl_wheel = \"zome_view\"").unwrap();
        assert!(matches!(
            config.actions.ctrl_wheel,
            CtrlWheelAction::ZoomView
        ));
    }

    #[test]
    fn load_from_missing_dir_returns_defaults() {
        let dir = std::env::temp_dir().join(format!("ktfm-test-load1-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir); // 确保目录不存在

        let config = AppConfig::load_from(&dir).unwrap();

        assert!(matches!(config.view.view_mode, view::ViewMode::List));
        assert!(!config.view.show_hidden_files, "未配置时应为默认值");
        assert!(dir.join(CONFIG_FILE_NAME).exists(), "应顺带落一份默认配置");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_reads_existing() {
        let dir = temp_dir("load2");
        fs::write(
            dir.join(CONFIG_FILE_NAME),
            "[view]\nshow_hidden_files = true\n",
        )
        .unwrap();

        let config = AppConfig::load_from(&dir).unwrap();

        assert!(config.view.show_hidden_files, "应读到用户配置的值");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_rejects_malformed() {
        let dir = temp_dir("load3");
        fs::write(dir.join(CONFIG_FILE_NAME), "not a [ valid toml").unwrap();

        let err = AppConfig::load_from(&dir).unwrap_err();

        assert!(
            matches!(err, Error::ConfigParse(_)),
            "应报解析错误，实际: {err:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_to_creates_dir_and_roundtrips() {
        let dir = temp_dir("save1").join("nested"); // 目录本身不存在，测自动创建

        let mut config = AppConfig::default();
        config.view.show_hidden_files = true;
        config.save_to(&dir).unwrap();

        let reloaded = AppConfig::load_from(&dir).unwrap();
        assert!(reloaded.view.show_hidden_files, "保存后应能读回修改的值");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }
}
