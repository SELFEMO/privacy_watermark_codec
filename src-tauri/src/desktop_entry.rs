#[cfg(target_os = "linux")]
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "linux")]
use tauri::AppHandle;

#[cfg(target_os = "linux")]
const APP_ID: &str = "com.privacywatermark.codec";
#[cfg(target_os = "linux")]
const DESKTOP_FILE_NAME: &str = "com.privacywatermark.codec.desktop";
#[cfg(target_os = "linux")]
const LEGACY_DESKTOP_FILE_NAME: &str = "privacy-watermark-codec.desktop";
#[cfg(target_os = "linux")]
const LEGACY_WINDOW_CLASS: &str = "privacy-watermark-codec";
#[cfg(target_os = "linux")]
const ICON_STEM: &str = "com.privacywatermark.codec";
#[cfg(target_os = "linux")]
const APP_NAME: &str = "图隐私水印编解码器";
#[cfg(target_os = "linux")]
const APP_COMMENT: &str = "Local invisible privacy watermark encoder and decoder";

#[cfg(target_os = "linux")]
pub fn ensure_linux_desktop_entry(_app: &AppHandle) {
    if let Err(error) = write_linux_desktop_entry() {
        tracing::warn!(%error, "无法写入 Linux 应用入口，Dock 图标可能继续显示为未知软件");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_linux_desktop_entry(_app: &tauri::AppHandle) {}

#[cfg(target_os = "linux")]
fn write_linux_desktop_entry() -> io::Result<()> {
    let Some(home_dir) = env::var_os("HOME").map(PathBuf::from) else {
        return Ok(());
    };

    let applications_dir = home_dir.join(".local/share/applications");
    fs::create_dir_all(&applications_dir)?;
    let icon_paths = write_linux_icons(&home_dir)?;

    let executable = env::current_exe()?;
    let quoted_executable = quote_desktop_exec_path(&executable);
    let desktop_path = applications_dir.join(DESKTOP_FILE_NAME);
    let desktop_entry = desktop_entry_text(&quoted_executable, APP_ID, false);
    // 桌面文件名、StartupWMClass 和 tauri.conf.json 的 GTK app id 必须同名，GNOME 才能把运行窗口归并到带应用图标的启动器项。
    // The desktop filename, StartupWMClass, and GTK app id in tauri.conf.json must share the same name so GNOME can group the running window with the launcher icon.
    write_text_if_changed(&desktop_path, &desktop_entry)?;

    let legacy_desktop_path = applications_dir.join(LEGACY_DESKTOP_FILE_NAME);
    let legacy_desktop_entry = desktop_entry_text(&quoted_executable, LEGACY_WINDOW_CLASS, true);
    // 兼容入口只用于匹配旧的 X11 WM_CLASS，不出现在应用菜单中；这样即使发行版没有立即采用 GTK app id，Dock 也能命中同一个图标。
    // The compatibility entry only matches the old X11 WM_CLASS and stays hidden from menus, so the Dock can still hit the same icon if the desktop has not adopted the GTK app id immediately.
    write_text_if_changed(&legacy_desktop_path, &legacy_desktop_entry)?;

    refresh_desktop_caches(&home_dir, &applications_dir);

    tracing::info!(desktop = %desktop_path.display(), legacy_desktop = %legacy_desktop_path.display(), icons = ?icon_paths, "已确保 Linux Dock 桌面入口");
    Ok(())
}



#[cfg(target_os = "linux")]
fn refresh_desktop_caches(home_dir: &Path, applications_dir: &Path) {
    // 缓存刷新失败不应阻止程序启动；它只是帮助桌面环境更快发现新的图标和入口。
    // Cache refresh failures must not block app startup; this only helps the desktop environment notice the new icon and entry sooner.
    let _ = Command::new("gtk-update-icon-cache")
        .arg("-q")
        .arg(home_dir.join(".local/share/icons/hicolor"))
        .status();
    let _ = Command::new("update-desktop-database")
        .arg(applications_dir)
        .status();
}

#[cfg(target_os = "linux")]
fn write_linux_icons(home_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let themed_icons = [
        ("32x32/apps", format!("{ICON_STEM}.png"), include_bytes!("../icons/32x32.png").as_slice()),
        ("128x128/apps", format!("{ICON_STEM}.png"), include_bytes!("../icons/128x128.png").as_slice()),
        ("256x256/apps", format!("{ICON_STEM}.png"), include_bytes!("../icons/128x128@2x.png").as_slice()),
        ("scalable/apps", format!("{ICON_STEM}.svg"), include_bytes!("../icons/icon.svg").as_slice()),
    ];
    let mut written_paths = Vec::with_capacity(themed_icons.len());
    for (theme_subdir, file_name, bytes) in themed_icons {
        let icon_dir = home_dir.join(".local/share/icons/hicolor").join(theme_subdir);
        fs::create_dir_all(&icon_dir)?;
        let icon_path = icon_dir.join(file_name);
        // GNOME Shell 按图标主题名和可用尺寸选择 Dock 图标；写入多尺寸 hicolor 资源可避免缩放或缓存场景回退到齿轮图标。
        // GNOME Shell chooses Dock icons by theme name and available sizes; writing multiple hicolor resources avoids a gear fallback during scaling or cache refreshes.
        write_bytes_if_changed(&icon_path, bytes)?;
        written_paths.push(icon_path);
    }
    Ok(written_paths)
}

#[cfg(target_os = "linux")]
fn desktop_entry_text(quoted_executable: &str, startup_class: &str, no_display: bool) -> String {
    let no_display_line = if no_display { "NoDisplay=true\n" } else { "" };
    format!(
        "[Desktop Entry]\nType=Application\nName={APP_NAME}\nComment={APP_COMMENT}\nExec={quoted_executable}\nIcon={APP_ID}\nTerminal=false\nCategories=Utility;\nStartupNotify=true\nStartupWMClass={startup_class}\nX-GNOME-WMClass={startup_class}\n{no_display_line}"
    )
}

#[cfg(target_os = "linux")]
fn write_bytes_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).map(|existing| existing == bytes).unwrap_or(false) {
        return Ok(());
    }
    fs::write(path, bytes)
}

#[cfg(target_os = "linux")]
fn write_text_if_changed(path: &Path, text: &str) -> io::Result<()> {
    if fs::read_to_string(path).map(|existing| existing == text).unwrap_or(false) {
        return Ok(());
    }
    fs::write(path, text)
}

#[cfg(target_os = "linux")]
fn quote_desktop_exec_path(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!("\"{escaped}\"")
}
