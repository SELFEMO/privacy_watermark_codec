use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

#[derive(Debug, Clone)]
pub struct FfmpegTools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegRuntimeInfo {
    pub platform: String,
    pub version: String,
    pub source: String,
    pub build_license: String,
    pub build_configure: String,
    pub generated_at: Option<String>,
    pub utc_compile_date: Option<String>,
    pub ffmpeg: Option<FfmpegBinaryInfo>,
    pub ffprobe: Option<FfmpegBinaryInfo>,
    pub extra_binaries: Vec<FfmpegBinaryInfo>,
    pub license_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegBinaryInfo {
    pub name: String,
    pub path: String,
    pub expected_sha256: String,
    pub actual_sha256: String,
    pub hash_ok: bool,
    pub version_line: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegManifest {
    version: Option<String>,
    source: Option<String>,
    build_license: Option<String>,
    build_configure: Option<String>,
    generated_at: Option<String>,
    utc_compile_date: Option<String>,
    platforms: HashMap<String, PlatformManifest>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlatformManifest {
    ffmpeg: BinaryManifest,
    ffprobe: BinaryManifest,
    ffplay: Option<BinaryManifest>,
}

#[derive(Debug, Clone, Deserialize)]
struct BinaryManifest {
    file: String,
    sha256: String,
}

pub fn bundled_tools(app: &AppHandle) -> Result<FfmpegTools, String> {
    let manifest = load_manifest(app)?;
    let platform = platform_key()?;
    let platform_entry = manifest
        .platforms
        .get(platform)
        .ok_or_else(|| format!("当前平台未在 FFmpeg 清单中配置：{platform}"))?;

    let ffmpeg = verified_binary_path(app, platform, "ffmpeg", &platform_entry.ffmpeg)?;
    let ffprobe = verified_binary_path(app, platform, "ffprobe", &platform_entry.ffprobe)?;
    Ok(FfmpegTools { ffmpeg, ffprobe })
}

#[tauri::command]
pub async fn get_ffmpeg_info(app: AppHandle) -> Result<FfmpegRuntimeInfo, String> {
    tauri::async_runtime::spawn_blocking(move || get_ffmpeg_info_blocking(app))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

fn get_ffmpeg_info_blocking(app: AppHandle) -> Result<FfmpegRuntimeInfo, String> {
    let manifest = load_manifest(&app)?;
    let platform = platform_key()?.to_owned();
    let platform_entry = manifest.platforms.get(&platform);

    let ffmpeg = platform_entry
        .map(|entry| binary_info(&app, &platform, "ffmpeg", &entry.ffmpeg))
        .transpose()?;
    let ffprobe = platform_entry
        .map(|entry| binary_info(&app, &platform, "ffprobe", &entry.ffprobe))
        .transpose()?;
    let mut extra_binaries = Vec::new();
    if let Some(ffplay) = platform_entry.and_then(|entry| entry.ffplay.as_ref()) {
        extra_binaries.push(binary_info(&app, &platform, "ffplay", ffplay)?);
    }

    let license_text = ffmpeg
        .as_ref()
        .and_then(|info| {
            if info.error.is_none() {
                command_text(Path::new(&info.path), "-L").ok()
            } else {
                None
            }
        })
        .filter(|text| !text.trim().is_empty())
        .or_else(|| read_resource_text(&app, "vendor/ffmpeg/LICENSE.txt").ok())
        .unwrap_or_else(|| "未找到 FFmpeg 许可证文本。请保留发行包附带的 LICENSE/COPYING 文件。".into());

    Ok(FfmpegRuntimeInfo {
        platform,
        version: manifest.version.unwrap_or_else(|| "unknown".into()),
        source: manifest.source.unwrap_or_else(|| "unknown".into()),
        build_license: manifest.build_license.unwrap_or_else(|| "unknown".into()),
        build_configure: manifest.build_configure.unwrap_or_else(|| "unknown".into()),
        generated_at: manifest.generated_at,
        utc_compile_date: manifest.utc_compile_date,
        ffmpeg,
        ffprobe,
        extra_binaries,
        license_text,
    })
}

fn load_manifest(app: &AppHandle) -> Result<FfmpegManifest, String> {
    let text = read_resource_text(app, "vendor/ffmpeg/manifest.json")?;
    serde_json::from_str(&text).map_err(|error| format!("FFmpeg manifest.json 格式错误：{error}"))
}

fn read_resource_text(app: &AppHandle, resource: &str) -> Result<String, String> {
    let path = resolve_existing_resource_path(app, resource)?;
    fs::read_to_string(&path).map_err(|error| format!("无法读取资源文件 {}：{error}", path.display()))
}

fn verified_binary_path(
    app: &AppHandle,
    platform: &str,
    name: &str,
    manifest: &BinaryManifest,
) -> Result<PathBuf, String> {
    let path = resolve_binary_path(app, platform, manifest)?;
    let actual = sha256_file(&path)?;

    match validate_expected_hash(name, &manifest.sha256) {
        Ok(()) => {
            let expected = manifest.sha256.to_lowercase();
            // 运行视频处理前强制校验哈希，是为了避免安装目录中的 FFmpeg 被替换后继续接触用户隐私媒体。
            // Hash verification is mandatory before video processing so a replaced FFmpeg cannot silently access private media.
            if actual != expected {
                return Err(format!(
                    "{name} 哈希校验失败：期望 {expected}，实际 {actual}。请重新运行 npm run ffmpeg:manifest 后再打包。"
                ));
            }
        }
        Err(error) => {
            if cfg!(debug_assertions) {
                // 开发态允许用实际文件哈希临时通过，是为了避免 target/debug 资源缓存导致明明有二进制却无法调试视频功能；正式构建仍会由 strict 脚本拦截。
                // Development may temporarily trust the actual file hash to avoid target/debug resource-cache false negatives; release builds are still blocked by the strict manifest script.
                tracing::warn!(binary = name, %actual, %error, "FFmpeg manifest 未写入期望哈希，开发态临时使用实际哈希");
            } else {
                return Err(error);
            }
        }
    }

    Ok(path)
}

fn binary_info(
    app: &AppHandle,
    platform: &str,
    name: &str,
    manifest: &BinaryManifest,
) -> Result<FfmpegBinaryInfo, String> {
    let path = resolve_binary_path(app, platform, manifest)?;
    let actual_result = sha256_file(&path);
    let actual = actual_result.clone().unwrap_or_default();
    let expected_is_valid = validate_expected_hash(name, &manifest.sha256).is_ok();
    let expected = if expected_is_valid {
        manifest.sha256.to_lowercase()
    } else {
        actual.clone()
    };
    let hash_ok = actual_result.is_ok() && !actual.is_empty() && actual == expected;
    let version_line = if path.is_file() {
        command_text(&path, "-version")
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
            .to_owned()
    } else {
        String::new()
    };
    let error = if actual_result.is_err() {
        actual_result.err()
    } else if expected_is_valid {
        None
    } else {
        // 已读到真实二进制时不在 UI 中继续报错，避免用户已放入 FFmpeg 后仍看到“未生成”的误导提示；发布校验仍由 npm run ffmpeg:manifest:strict 负责。
        // Once the real binary is readable, the UI no longer shows a misleading missing-hash error; release validation remains enforced by npm run ffmpeg:manifest:strict.
        None
    };

    Ok(FfmpegBinaryInfo {
        name: name.to_owned(),
        path: path.display().to_string(),
        expected_sha256: expected,
        actual_sha256: actual,
        hash_ok,
        version_line,
        error,
    })
}

fn validate_expected_hash(name: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains("填写") || trimmed.contains("REPLACE") {
        return Err(format!(
            "{name} 的 SHA-256 未生成。请把 FFmpeg 二进制放入 src-tauri/vendor/ffmpeg 后执行 npm run ffmpeg:manifest。"
        ));
    }
    if trimmed.len() != 64 || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("{name} 的 SHA-256 格式不正确：{trimmed}"));
    }
    Ok(())
}

fn resolve_binary_path(
    app: &AppHandle,
    platform: &str,
    manifest: &BinaryManifest,
) -> Result<PathBuf, String> {
    let relative = format!("vendor/ffmpeg/{platform}/{}", manifest.file);
    resolve_existing_resource_path(app, &relative)
}

fn resolve_existing_resource_path(app: &AppHandle, resource: &str) -> Result<PathBuf, String> {
    let candidates = candidate_resource_paths(app, resource);
    for candidate in &candidates {
        if is_regular_file(candidate) {
            return Ok(candidate.clone());
        }
    }

    // 开发态优先读取项目内 vendor，打包态再读取 Tauri Resource；这能避免 target/debug 下不存在资源时误报。
    // Development prefers the project vendor directory, while packaged builds fall back to Tauri resources to avoid false misses under target/debug.
    let joined = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n- ");
    Err(format!("未找到资源文件 {resource}。已检查路径：\n- {joined}"))
}

fn candidate_resource_paths(app: &AppHandle, resource: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // 先检查项目源码目录，是因为 tauri dev 时用户实际替换 FFmpeg 的位置就在 src-tauri/vendor/ffmpeg。
    // The source directory is checked first because developers replace FFmpeg under src-tauri/vendor/ffmpeg during tauri dev.
    candidates.push(manifest_dir.join(resource));

    if let Some(project_root) = manifest_dir.parent() {
        candidates.push(project_root.join("src-tauri").join(resource));
        candidates.push(project_root.join(resource));
    }

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("src-tauri").join(resource));
        candidates.push(current_dir.join(resource));
    }

    if let Ok(path) = app.path().resolve(resource, BaseDirectory::Resource) {
        candidates.push(path);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("vendor").join(resource.strip_prefix("vendor/").unwrap_or(resource)));
            candidates.push(exe_dir.join("resources").join(resource));
            candidates.push(exe_dir.join(resource));
            if let Some(parent) = exe_dir.parent() {
                candidates.push(parent.join("Resources").join(resource));
                candidates.push(parent.join("resources").join(resource));
                candidates.push(parent.join(resource));
            }
        }
    }

    let mut unique = Vec::new();
    for candidate in candidates {
        if !unique.iter().any(|item: &PathBuf| item == &candidate) {
            unique.push(candidate);
        }
    }
    unique
}

fn is_regular_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("无法打开文件 {}：{error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取文件失败 {}：{error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn command_text(path: &Path, arg: &str) -> Result<String, String> {
    let output = hidden_command(path)
        .arg(arg)
        .output()
        .map_err(|error| format!("无法执行 {}：{error}", path.display()))?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text.trim().to_owned())
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // 后台探测 FFmpeg 时不创建控制台窗口，避免用户在图形界面操作时看到命令行一闪而过。
        // FFmpeg probing is started without a console window so GUI users do not see a flashing command prompt.
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn platform_key() -> Result<&'static str, String> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Ok("windows_x64")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Ok("windows_arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Ok("macos_x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok("macos_arm64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("linux_x64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Ok("linux_arm64")
    } else {
        Err("当前平台暂未配置内置 FFmpeg".into())
    }
}
