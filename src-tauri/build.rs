use std::{env, fs, io, path::{Path, PathBuf}};

fn main() {
    println!("cargo:rerun-if-changed=vendor/ffmpeg");
    println!("cargo:rerun-if-changed=icons/128x128.rgba");
    println!("cargo:rerun-if-changed=icons/icon.ico");

    if let Err(error) = mirror_ffmpeg_vendor_to_target() {
        println!("cargo:warning=Failed to mirror bundled FFmpeg resources: {error}");
    }

    tauri_build::build()
}

fn mirror_ffmpeg_vendor_to_target() -> io::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));
    let source = manifest_dir.join("vendor").join("ffmpeg");
    if !source.exists() {
        return Ok(());
    }

    let Some(profile_dir) = target_profile_dir()? else {
        return Ok(());
    };
    let destination = profile_dir.join("vendor").join("ffmpeg");
    fs::create_dir_all(&destination)?;

    for file in ["manifest.json", "LICENSE.txt", "README.md", "VERSION.txt"] {
        copy_if_exists(&source.join(file), &destination.join(file))?;
    }

    for platform in target_platform_keys() {
        let platform_source = source.join(platform);
        if platform_source.exists() {
            // 构建脚本只复制目标平台相关运行时，避免跨平台打包时把无关 FFmpeg 二进制塞进当前产物。
            // The build script copies only target-platform runtime files so cross-platform packages do not carry unrelated FFmpeg binaries.
            copy_dir_recursive(&platform_source, &destination.join(platform))?;
        }
    }

    Ok(())
}

fn target_platform_keys() -> Vec<&'static str> {
    match (
        env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str(),
        env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str(),
    ) {
        ("windows", "aarch64") => vec!["windows_arm64"],
        ("windows", _) => vec!["windows_x64"],
        ("macos", "aarch64") => vec!["macos_arm64"],
        // amd64 与 x64 是同一 Intel Mac 架构的两种常见命名，两个目录都复制可以兼容旧资源目录和新命名。
        // amd64 and x64 are two common names for the same Intel Mac architecture, so copying both directories keeps old and new resource layouts compatible.
        ("macos", _) => vec!["macos_amd64", "macos_x64"],
        ("linux", "aarch64") => vec!["linux_arm64"],
        ("linux", _) => vec!["linux_x64", "linux_amd64"],
        _ => vec!["windows_x64"],
    }
}

fn target_profile_dir() -> io::Result<Option<PathBuf>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap_or_default());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    for ancestor in out_dir.ancestors() {
        if ancestor.file_name().and_then(|name| name.to_str()) == Some(profile.as_str()) {
            return Ok(Some(ancestor.to_path_buf()));
        }
    }
    Ok(None)
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            copy_if_exists(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn copy_if_exists(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if should_copy(source, destination)? {
        make_destination_writable(destination)?;
        fs::copy(source, destination)?;
        ensure_runtime_file_permissions(destination)?;
    }
    Ok(())
}

fn make_destination_writable(destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(destination)?;
        let mode = metadata.permissions().mode();
        if mode & 0o200 == 0 {
            let mut permissions = metadata.permissions();
            // target 目录中的 FFmpeg 可能来自上一轮只读拷贝；覆盖前先恢复 owner 写入位，避免 build.rs 在 macOS 上 Permission denied。
            // FFmpeg in target may come from an earlier read-only copy; owner write permission is restored before overwriting to avoid Permission denied on macOS.
            permissions.set_mode(mode | 0o200);
            fs::set_permissions(destination, permissions)?;
        }
    }
    Ok(())
}

fn ensure_runtime_file_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return Ok(());
        };
        if matches!(file_name, "ffmpeg" | "ffprobe" | "ffplay") {
            let metadata = fs::metadata(path)?;
            let mode = metadata.permissions().mode();
            if mode & 0o755 != 0o755 {
                let mut permissions = metadata.permissions();
                // 复制到 target 后补齐执行位和 owner 写入位，保证开发热重载与后续复制都能继续覆盖该文件。
                // After copying into target, executable and owner-write bits are restored so dev reloads and later copies can overwrite the file.
                permissions.set_mode(mode | 0o755);
                fs::set_permissions(path, permissions)?;
            }
        }
    }
    Ok(())
}

fn should_copy(source: &Path, destination: &Path) -> io::Result<bool> {
    if !destination.exists() {
        return Ok(true);
    }
    let source_meta = fs::metadata(source)?;
    let destination_meta = fs::metadata(destination)?;
    Ok(source_meta.len() != destination_meta.len()
        || source_meta.modified().ok() != destination_meta.modified().ok())
}
