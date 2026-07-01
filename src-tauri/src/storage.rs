use std::{
    fs,
    io,
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
    let storage = exe_dir.join(STORAGE_DIR_NAME);
    // 用户明确要求缓存与运行数据不进入 %APPDATA%，因此所有运行期数据都固定放在可执行文件旁的专用目录中。
    // Runtime data is pinned beside the executable because the user explicitly does not want caches or state in %APPDATA%.
    fs::create_dir_all(&storage).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "无法在软件目录创建运行数据目录：{}。请确认安装目录可写，或将软件安装到非系统盘可写目录。原始错误：{error}",
                storage.display()
            ),
        )
    })?;
    Ok(storage)
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
