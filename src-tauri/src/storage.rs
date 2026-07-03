use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const STORAGE_DIR_NAME: &str = "PrivacyWatermarkCodecData";
const WEBVIEW_DIR_NAME: &str = "webview-data";
const WORK_DIR_NAME: &str = "work";

static NEXT_WORK_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct AppWorkDir {
    path: PathBuf,
}

impl AppWorkDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AppWorkDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            tracing::warn!(path = %self.path.display(), %error, "清理软件目录内临时工作区失败");
        }
    }
}

pub fn storage_root() -> io::Result<PathBuf> {
    let exe_dir = executable_directory()?;
    let portable_storage = exe_dir.join(STORAGE_DIR_NAME);
    match ensure_writable_directory(&portable_storage) {
        Ok(()) => Ok(portable_storage),
        Err(portable_error) => fallback_storage_root(&portable_storage, portable_error),
    }
}

fn fallback_storage_root(portable_storage: &Path, portable_error: io::Error) -> io::Result<PathBuf> {
    let Some(user_storage) = user_data_storage_root() else {
        return Err(io::Error::new(
            portable_error.kind(),
            format!(
                "无法在软件目录创建运行数据目录：{}。请确认安装目录可写，或使用 deb/rpm 安装后从终端运行以查看权限错误。原始错误：{}",
                portable_storage.display(),
                portable_error
            ),
        ));
    };

    match ensure_writable_directory(&user_storage) {
        Ok(()) => {
            // deb/rpm 安装后的可执行文件通常位于系统目录，普通用户无法在旁边写入；只在旁路目录不可写时回退到用户数据目录，才能避免启动阶段闪退。
            // deb/rpm installs usually place the executable in system directories that normal users cannot write beside; falling back only after that failure prevents startup crashes.
            Ok(user_storage)
        }
        Err(user_error) => Err(io::Error::new(
            user_error.kind(),
            format!(
                "无法创建运行数据目录。已尝试软件目录 {} 和用户目录 {}。软件目录错误：{}；用户目录错误：{}",
                portable_storage.display(),
                user_storage.display(),
                portable_error,
                user_error
            ),
        )),
    }
}

fn ensure_writable_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let probe = path.join(format!(
        ".write-test-{}-{}",
        std::process::id(),
        NEXT_WORK_ID.fetch_add(1, Ordering::Relaxed)
    ));
    // create_dir_all 在目录已存在但不可写时可能不会暴露问题；写入探针文件能提前发现 deb/rpm 系统目录权限错误。
    // create_dir_all may hide permission problems when the directory already exists; a probe file detects deb/rpm system-directory write failures early.
    fs::write(&probe, b"ok")?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

fn user_data_storage_root() -> Option<PathBuf> {
    let base = user_data_base_directory()?;
    // 回退目录只在软件旁目录不可写时使用，因此仍保留可移动/开发态的旁路存储习惯，同时保证系统安装包能正常启动。
    // The fallback is used only when beside-executable storage is not writable, preserving portable/dev storage while keeping system-installed packages launchable.
    Some(base.join("privacy-watermark-codec").join(STORAGE_DIR_NAME))
}

#[cfg(target_os = "linux")]
fn user_data_base_directory() -> Option<PathBuf> {
    env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share"))
        })
}

#[cfg(target_os = "macos")]
fn user_data_base_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join("Library/Application Support"))
}

#[cfg(target_os = "windows")]
fn user_data_base_directory() -> Option<PathBuf> {
    env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join("AppData/Local"))
        })
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn user_data_base_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn webview_data_directory() -> io::Result<PathBuf> {
    let path = storage_root()?.join(WEBVIEW_DIR_NAME);
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn create_work_dir(prefix: &str) -> io::Result<AppWorkDir> {
    let root = storage_root()?.join(WORK_DIR_NAME);
    fs::create_dir_all(&root)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let id = NEXT_WORK_ID.fetch_add(1, Ordering::Relaxed);
    let dir = root.join(format!("{prefix}_{}_{}", std::process::id(), now + u128::from(id)));

    // 每个视频任务使用独立工作区，避免并发任务互相覆盖帧文件，同时仍能在 Drop 中自动清理。
    // Each video task uses an isolated workspace to prevent frame-file collisions while still being cleaned on Drop.
    fs::create_dir_all(&dir)?;
    Ok(AppWorkDir { path: dir })
}

fn executable_directory() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法确定软件所在目录"))
}
