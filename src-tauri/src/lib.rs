mod cancellation;
mod commands;
mod evidence;
mod ffmpeg;
mod launch;
mod media;
mod models;
mod progress;
mod release;
mod storage;
mod video;

use tauri::{Emitter, Manager};
use tracing_subscriber::EnvFilter;

#[cfg(target_os = "linux")]
fn prefer_native_linux_display_backend() {
    let has_wayland_display = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let has_explicit_backend = std::env::var_os("GDK_BACKEND").is_some();

    if has_wayland_display && !has_explicit_backend {
        // GNOME/KDE 分数缩放下，GTK 走 XWayland 时常会被合成器按位图放大；优先使用原生 Wayland 可以减少整窗低分辨率拉伸。
        // Under GNOME/KDE fractional scaling, GTK through XWayland is often bitmap-scaled by the compositor; preferring native Wayland reduces whole-window low-resolution stretching.
        std::env::set_var("GDK_BACKEND", "wayland,x11");
    }
}

#[cfg(not(target_os = "linux"))]
fn prefer_native_linux_display_backend() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    prefer_native_linux_display_backend();

    // 日志仅写入启动应用的终端，不向前端发送，从而保持界面简洁且便于开发者排查处理细节。
    // Logs are written only to the launching terminal, not emitted to the frontend, keeping the UI clean while preserving diagnostics.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("privacy_watermark_codec=info,watermark_core=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_ansi(true)
        .try_init();

    tauri::Builder::default()
        .manage(cancellation::CancellationRegistry::default())
        .manage(launch::PendingLaunchContexts::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let context = launch::parse_launch_context_from_strings(args);
            launch::store_pending_launch_context(app, context.clone());
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.emit("pwc-launch-context", context);
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let webview_data_dir = storage::webview_data_directory()?;
            tracing::info!(path = %webview_data_dir.display(), "使用运行数据目录内的 WebView 数据目录");

            let main_window_config = app.config().app.windows.first().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "缺少 main 窗口配置，请检查 src-tauri/tauri.conf.json",
                )
            })?;
            let window_icon = tauri::image::Image::new(
                include_bytes!("../icons/256x256.rgba"),
                256,
                256,
            );

            // 主窗口仍然由 Rust 手动创建，是为了保留绝对 WebView data_directory；其它窗口身份配置放回 tauri.conf.json，避免 Linux Dock 匹配信息分散。
            // The main window is still created from Rust to keep the absolute WebView data_directory; the rest of the window identity stays in tauri.conf.json to avoid scattered Linux Dock matching data.
            tauri::WebviewWindowBuilder::from_config(app, main_window_config)?
                // 运行时窗口图标使用更高像素密度的 RGBA 数据，避免 HiDPI 标题栏或任务切换器把小图标放大成模糊位图。
                // The runtime window icon uses higher-density RGBA data so HiDPI title bars or task switchers do not upscale a small icon into a blurry bitmap.
                .icon(window_icon)?
                .data_directory(webview_data_dir)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::encode_media,
            commands::decode_media,
            commands::scan_privacy_watermark,
            commands::cancel_task,
            ffmpeg::get_ffmpeg_info,
            launch::get_launch_context,
            release::get_release_metadata
        ])
        .run(tauri::generate_context!())
        .expect("启动图隐私水印编解码器失败");
}
