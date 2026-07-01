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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            tracing::info!(path = %webview_data_dir.display(), "使用软件目录内的 WebView 数据目录");

            // 手动创建主窗口是为了给 WebView 指定绝对 data_directory，避免默认落入 %APPDATA% 或 LocalAppData。
            // The main window is created manually so WebView receives an absolute data_directory instead of defaulting to %APPDATA% or LocalAppData.
            let window_icon = tauri::image::Image::new(
                include_bytes!("../icons/128x128.rgba"),
                128,
                128,
            );

            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("图隐私水印编解码器")
            // 运行时窗口图标使用原始 RGBA 像素数据，可避免标题栏继续显示旧缓存图标。
            // The runtime window icon uses raw RGBA pixels so the title bar does not fall back to a stale cached icon.
            .icon(window_icon)?
            .inner_size(1280.0, 900.0)
            .min_inner_size(960.0, 720.0)
            .resizable(true)
            .fullscreen(false)
            .center()
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
