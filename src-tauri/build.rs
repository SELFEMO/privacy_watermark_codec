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

    let platform = target_platform_key();
    let platform_source = source.join(platform);
    if platform_source.exists() {
        copy_dir_recursive(&platform_source, &destination.join(platform))?;
    }

    Ok(())
}

fn target_platform_key() -> &'static str {
    match (
        env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str(),
        env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default().as_str(),
    ) {
        ("windows", "aarch64") => "windows_arm64",
        ("windows", _) => "windows_x64",
        ("macos", "aarch64") => "macos_arm64",
        ("macos", _) => "macos_x64",
        ("linux", "aarch64") => "linux_arm64",
        ("linux", _) => "linux_x64",
        _ => "windows_x64",
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
        fs::copy(source, destination)?;
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
