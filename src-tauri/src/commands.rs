use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Local;
use tracing::{error, info};
use watermark_core::{
    embed_image_file, extract_image_file, scan_image_file, EmbedOptions, KeyFile, KeyMode, KeySource,
    WatermarkKey,
};

use crate::{
    cancellation::{CancellationRegistry, CancellationToken},
    evidence::{file_sha256, write_evidence_manifest, EvidenceEntry},
    media::{detect_media_type, safe_stem},
    models::{
        CancelTaskRequest, DecodeItemResult, DecodeRequest, DecodeResponse, EncodeItemResult,
        EncodeRequest, EncodeResponse, MediaType, ScanItemResult, ScanRequest, ScanResponse,
        TaskProgressEvent,
        TaskProgressKind,
    },
    progress::{clamp_percent, emit_task_progress},
    release::current_release_metadata,
    video::{decode_video, encode_video, VideoProcessingOptions, VideoProgressContext},
};

#[tauri::command]
pub async fn encode_media(
    app: tauri::AppHandle,
    cancellation: tauri::State<'_, CancellationRegistry>,
    request: EncodeRequest,
) -> Result<EncodeResponse, String> {
    let registry = cancellation.inner().clone();
    tauri::async_runtime::spawn_blocking(move || encode_media_blocking(app, request, registry))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

#[tauri::command]
pub async fn decode_media(
    app: tauri::AppHandle,
    cancellation: tauri::State<'_, CancellationRegistry>,
    request: DecodeRequest,
) -> Result<DecodeResponse, String> {
    let registry = cancellation.inner().clone();
    tauri::async_runtime::spawn_blocking(move || decode_media_blocking(app, request, registry))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

#[tauri::command]
pub async fn scan_privacy_watermark(
    app: tauri::AppHandle,
    cancellation: tauri::State<'_, CancellationRegistry>,
    request: ScanRequest,
) -> Result<ScanResponse, String> {
    let registry = cancellation.inner().clone();
    tauri::async_runtime::spawn_blocking(move || scan_privacy_watermark_blocking(app, request, registry))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

#[tauri::command]
pub fn cancel_task(
    cancellation: tauri::State<'_, CancellationRegistry>,
    request: CancelTaskRequest,
) -> Result<(), String> {
    cancellation.request_cancel(&request.task_id);
    Ok(())
}

fn encode_media_blocking(
    app: tauri::AppHandle,
    request: EncodeRequest,
    cancellation: CancellationRegistry,
) -> Result<EncodeResponse, String> {
    validate_encode_request(&request)?;
    let output_base = PathBuf::from(&request.output_dir);
    fs::create_dir_all(&output_base).map_err(|error| error.to_string())?;

    let task_id = request.task_id.clone();
    let total_files = request.input_paths.len();
    let cancellation = CancellationToken::new(task_id.clone(), cancellation);
    info!(
        files = total_files,
        ?request.key_mode,
        output = %output_base.display(),
        "收到批量编码任务"
    );
    emit_progress(
        &app,
        task_id.clone(),
        TaskProgressKind::Encode,
        "preparing",
        "正在准备批量编码任务",
        0,
        total_files,
        0.0,
        None,
    );
    cancellation.check()?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let mut shared_key_path = None;
    // 批次级密钥只创建一次，保证共享模式可统一解码；独立模式则在文件循环中逐个创建以隔离风险。
    // A batch key is created once for shared decoding, while independent mode creates one per file to isolate compromise.
    let shared_key = match request.key_mode {
        KeyMode::Independent => None,
        KeyMode::Shared => Some(WatermarkKey::random(KeyMode::Shared)),
        KeyMode::Custom => Some(
            WatermarkKey::from_password(
                request.custom_password.as_deref().unwrap_or_default(),
                None,
            )
            .map_err(|error| error.to_string())?,
        ),
    };

    let output_root = match request.key_mode {
        KeyMode::Independent => output_base,
        KeyMode::Shared => output_base.join(format!("watermarked_batch_{timestamp}")),
        KeyMode::Custom => output_base.join(format!("custom_batch_{timestamp}")),
    };
    fs::create_dir_all(&output_root).map_err(|error| error.to_string())?;
    let mut output_root_guard = EmptyDirectoryGuard::new(
        output_root.clone(),
        request.key_mode != KeyMode::Independent,
    );
    cancellation.check()?;

    if let Some(key) = &shared_key {
        let should_write = request.key_mode != KeyMode::Custom || request.write_key_file;
        if should_write {
            emit_progress(
                &app,
                task_id.clone(),
                TaskProgressKind::Encode,
                "writing-key",
                "正在写入批次密钥文件",
                0,
                total_files,
                1.0,
                None,
            );
            let file_name = if request.key_mode == KeyMode::Shared {
                "shared.key"
            } else {
                "custom.key"
            };
            let key_path = output_root.join(file_name);
            key.to_key_file()
                .write(&key_path)
                .map_err(|error| error.to_string())?;
            // 先记录路径再转移所有权，避免 PathBuf 被移入 Option 后仍被日志借用。
            // The path is logged before ownership is moved into Option so PathBuf is not borrowed after move.
            info!(path = %key_path.display(), "已写入批次密钥文件");
            shared_key_path = Some(key_path);
            cancellation.check()?;
        }
    }

    let mut items = Vec::with_capacity(request.input_paths.len());
    let mut batch_entries = Vec::with_capacity(request.input_paths.len());
    let video_options = video_options_from_request(request.frame_parallelism);
    for (file_index, input_string) in request.input_paths.iter().enumerate() {
        cancellation.check()?;
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            return Err(format!("输入文件不存在：{}", input.display()));
        }
        let media_type = detect_media_type(&input)
            .ok_or_else(|| format!("不支持的文件类型：{}", input.display()))?;
        let stem = safe_stem(&input);
        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Encode,
            "preparing-file",
            format!("正在准备编码第 {}/{} 个文件", file_index + 1, total_files),
            file_index,
            total_files,
            0.0,
            &input,
        );
        cancellation.check()?;
        let original_sha256 = file_sha256(&input).map_err(|error| error.to_string())?;

        let (item_dir, key, key_path) = match request.key_mode {
            KeyMode::Independent => {
                let item_dir = output_root.join(format!("{stem}_watermarked"));
                fs::create_dir_all(&item_dir).map_err(|error| error.to_string())?;
                let key = WatermarkKey::random(KeyMode::Independent);
                let key_path = item_dir.join(format!("{stem}.key"));
                key.to_key_file()
                    .write(&key_path)
                    .map_err(|error| error.to_string())?;
                (item_dir, key, Some(key_path))
            }
            _ => (output_root.clone(), shared_key.clone().unwrap(), None),
        };

        let output = match media_type {
            MediaType::Image => item_dir.join(format!("{stem}_watermarked.png")),
            MediaType::Video => item_dir.join(format!("{stem}_watermarked.mp4")),
        };

        let (psnr, frame_count) = match media_type {
            MediaType::Image => {
                emit_file_progress(
                    &app,
                    task_id.clone(),
                    TaskProgressKind::Encode,
                    "processing-image",
                    format!("正在嵌入图片水印：{}", input.display()),
                    file_index,
                    total_files,
                    0.45,
                    &input,
                );
                cancellation.check()?;
                let report = embed_image_file(
                    &input,
                    &output,
                    &EmbedOptions {
                        text: request.text.clone(),
                        key: key.clone(),
                        strength: request.strength,
                        media_kind: "image".into(),
                    },
                )
                .map_err(|error| error.to_string())?;
                (Some(report.psnr), None)
            }
            MediaType::Video => {
                let report = encode_video(
                    &app,
                    &input,
                    &output,
                    &request.text,
                    &key,
                    request.strength,
                    video_options,
                    &cancellation,
                    VideoProgressContext {
                        task_id: task_id.clone(),
                        task: TaskProgressKind::Encode,
                        file_index,
                        file_total: total_files,
                        file_path: input.display().to_string(),
                    },
                )?;
                (None, Some(report.frame_count))
            }
        };

        cancellation.check()?;
        let effective_key_path = key_path.as_deref().or(shared_key_path.as_deref());
        let key_file_sha256 = effective_key_path
            .map(file_sha256)
            .transpose()
            .map_err(|error| error.to_string())?;
        let entry = EvidenceEntry {
            input_path: input.display().to_string(),
            output_path: output.display().to_string(),
            media_type,
            original_sha256,
            output_sha256: file_sha256(&output).map_err(|error| error.to_string())?,
            key_file_sha256,
            psnr,
            frame_count,
        };

        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Encode,
            "writing-evidence",
            format!("正在写入证据清单：{}", output.display()),
            file_index,
            total_files,
            0.92,
            &input,
        );

        let manifest_path = if request.key_mode == KeyMode::Independent {
            let manifest_path = item_dir.join(format!("{stem}_evidence_manifest.json"));
            write_evidence_manifest(
                &manifest_path,
                vec![entry.clone()],
                &key,
                current_release_metadata(),
            )
            .map_err(|error| error.to_string())?;
            Some(manifest_path)
        } else {
            batch_entries.push(entry);
            None
        };

        items.push(EncodeItemResult {
            input_path: input.display().to_string(),
            output_path: output.display().to_string(),
            key_path: key_path.map(|path| path.display().to_string()),
            manifest_path: manifest_path.map(|path| path.display().to_string()),
            media_type,
            psnr,
            frame_count,
        });
        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Encode,
            "completed-file",
            format!("第 {}/{} 个文件编码完成", file_index + 1, total_files),
            file_index,
            total_files,
            1.0,
            &input,
        );
    }

    let manifest_path = if request.key_mode == KeyMode::Independent {
        None
    } else {
        let path = output_root.join("batch_evidence_manifest.json");
        let signing_key = shared_key.as_ref().unwrap();
        write_evidence_manifest(&path, batch_entries, signing_key, current_release_metadata())
            .map_err(|error| error.to_string())?;
        Some(path)
    };

    output_root_guard.disarm();

    emit_progress(
        &app,
        task_id,
        TaskProgressKind::Encode,
        "completed",
        "批量编码任务完成",
        total_files,
        total_files,
        100.0,
        None,
    );
    // 批量清单在所有输出完成后统一写入，确保原文件哈希、输出文件哈希和可选密钥文件哈希处于同一证据快照。
    // The batch manifest is written after all outputs finish so original, output, and optional key-file hashes represent one evidence snapshot.
    info!(completed = items.len(), "批量编码任务完成");
    Ok(EncodeResponse {
        output_root: output_root.display().to_string(),
        items,
        shared_key_path: shared_key_path.map(|path| path.display().to_string()),
        manifest_path: manifest_path.map(|path| path.display().to_string()),
    })
}

fn scan_privacy_watermark_blocking(
    app: tauri::AppHandle,
    request: ScanRequest,
    cancellation: CancellationRegistry,
) -> Result<ScanResponse, String> {
    if request.input_paths.is_empty() {
        return Err("请至少选择一张待检测图片".into());
    }
    let task_id = request.task_id.clone();
    let total_files = request.input_paths.len();
    let cancellation = CancellationToken::new(task_id.clone(), cancellation);
    info!(files = total_files, "收到未知来源图片隐私水印扫描任务");
    emit_progress(
        &app,
        task_id.clone(),
        TaskProgressKind::Scan,
        "preparing",
        "正在准备未知来源图片扫描",
        0,
        total_files,
        0.0,
        None,
    );
    cancellation.check()?;

    let mut items = Vec::with_capacity(request.input_paths.len());
    for (file_index, input_string) in request.input_paths.iter().enumerate() {
        cancellation.check()?;
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            return Err(format!("输入文件不存在：{}", input.display()));
        }
        let media_type = detect_media_type(&input)
            .ok_or_else(|| format!("不支持的文件类型：{}", input.display()))?;
        if media_type != MediaType::Image {
            return Err(format!("未知来源隐私水印扫描当前仅支持图片：{}", input.display()));
        }
        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Scan,
            "scanning-image",
            format!("正在扫描第 {}/{} 张图片", file_index + 1, total_files),
            file_index,
            total_files,
            0.50,
            &input,
        );

        cancellation.check()?;
        // 该扫描入口不要求用户提供密钥，用于先判断“是否有水印痕迹”；真正的加密正文仍交给解码入口处理。
        // This scan path does not require a key and only checks whether watermark traces exist; encrypted bodies are still handled by the decode path.
        let report = scan_image_file(&input).map_err(|error| {
            error!(input = %input.display(), %error, "未知来源图片扫描失败");
            error.to_string()
        })?;
        items.push(ScanItemResult {
            input_path: input.display().to_string(),
            status: report.status,
            summary: report.summary,
            detections: report.detections,
            width: report.width,
            height: report.height,
        });
        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Scan,
            "completed-file",
            format!("第 {}/{} 张图片扫描完成", file_index + 1, total_files),
            file_index,
            total_files,
            1.0,
            &input,
        );
    }

    emit_progress(
        &app,
        task_id,
        TaskProgressKind::Scan,
        "completed",
        "未知来源图片扫描完成",
        total_files,
        total_files,
        100.0,
        None,
    );
    info!(completed = items.len(), "未知来源图片隐私水印扫描完成");
    Ok(ScanResponse { items })
}

fn decode_media_blocking(
    app: tauri::AppHandle,
    request: DecodeRequest,
    cancellation: CancellationRegistry,
) -> Result<DecodeResponse, String> {
    if request.input_paths.is_empty() {
        return Err("请至少选择一个待解码媒体文件".into());
    }
    let key_source = build_key_source(&request)?;
    let video_options = video_options_from_request(request.frame_parallelism);
    let task_id = request.task_id.clone();
    let total_files = request.input_paths.len();
    let cancellation = CancellationToken::new(task_id.clone(), cancellation);
    info!(files = total_files, "收到批量解码任务");
    emit_progress(
        &app,
        task_id.clone(),
        TaskProgressKind::Decode,
        "preparing",
        "正在准备批量解码检测任务",
        0,
        total_files,
        0.0,
        None,
    );
    cancellation.check()?;

    let mut items = Vec::with_capacity(request.input_paths.len());
    for (file_index, input_string) in request.input_paths.iter().enumerate() {
        cancellation.check()?;
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            return Err(format!("输入文件不存在：{}", input.display()));
        }
        let media_type = detect_media_type(&input)
            .ok_or_else(|| format!("不支持的文件类型：{}", input.display()))?;
        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Decode,
            "preparing-file",
            format!("正在准备检测第 {}/{} 个文件", file_index + 1, total_files),
            file_index,
            total_files,
            0.0,
            &input,
        );
        cancellation.check()?;

        let result = match media_type {
            MediaType::Image => {
                emit_file_progress(
                    &app,
                    task_id.clone(),
                    TaskProgressKind::Decode,
                    "decoding-image",
                    format!("正在解码并检测图片：{}", input.display()),
                    file_index,
                    total_files,
                    0.50,
                    &input,
                );
                cancellation.check()?;
                let report = extract_image_file(&input, &key_source).map_err(|error| {
                    error!(input = %input.display(), %error, "图片解码失败");
                    error.to_string()
                })?;
                DecodeItemResult {
                    input_path: input.display().to_string(),
                    media_type,
                    text: report.text,
                    integrity: report.integrity,
                    fingerprint_distance: Some(report.fingerprint_distance),
                    corrected_codewords: report.corrected_codewords,
                    frame_count: None,
                    valid_frames: None,
                    modified_frames: None,
                    tamper_regions: report.tamper_regions,
                    sync_registration: Some(report.sync_registration),
                }
            }
            MediaType::Video => {
                let report = decode_video(
                    &app,
                    &input,
                    &key_source,
                    video_options,
                    &cancellation,
                    VideoProgressContext {
                        task_id: task_id.clone(),
                        task: TaskProgressKind::Decode,
                        file_index,
                        file_total: total_files,
                        file_path: input.display().to_string(),
                    },
                )?;
                DecodeItemResult {
                    input_path: input.display().to_string(),
                    media_type,
                    text: report.text,
                    integrity: report.integrity,
                    fingerprint_distance: None,
                    corrected_codewords: report.corrected_codewords,
                    frame_count: Some(report.frame_count),
                    valid_frames: Some(report.valid_frames),
                    modified_frames: Some(report.modified_frames),
                    tamper_regions: report.tamper_regions,
                    sync_registration: report.sync_registration,
                }
            }
        };
        items.push(result);
        emit_file_progress(
            &app,
            task_id.clone(),
            TaskProgressKind::Decode,
            "completed-file",
            format!("第 {}/{} 个文件解码检测完成", file_index + 1, total_files),
            file_index,
            total_files,
            1.0,
            &input,
        );
    }

    emit_progress(
        &app,
        task_id,
        TaskProgressKind::Decode,
        "completed",
        "批量解码检测任务完成",
        total_files,
        total_files,
        100.0,
        None,
    );
    info!(completed = items.len(), "批量解码任务完成");
    Ok(DecodeResponse { items })
}

struct EmptyDirectoryGuard {
    path: PathBuf,
    active: bool,
}

impl EmptyDirectoryGuard {
    fn new(path: PathBuf, active: bool) -> Self {
        Self { path, active }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for EmptyDirectoryGuard {
    fn drop(&mut self) {
        if self.active {
            // 只尝试删除空目录，避免任务失败时误删已经成功生成的媒体、密钥或证据文件。
            // Only empty directories are removed so a failed task never deletes media, key, or evidence files that were already produced.
            let _ = fs::remove_dir(&self.path);
        }
    }
}

fn emit_progress(
    app: &tauri::AppHandle,
    task_id: Option<String>,
    task: TaskProgressKind,
    phase: &str,
    message: impl Into<String>,
    current: usize,
    total: usize,
    percent: f64,
    current_path: Option<String>,
) {
    emit_task_progress(
        app,
        TaskProgressEvent {
            task_id,
            task,
            phase: phase.to_owned(),
            message: message.into(),
            current,
            total,
            percent: clamp_percent(percent),
            current_path,
        },
    );
}

fn emit_file_progress(
    app: &tauri::AppHandle,
    task_id: Option<String>,
    task: TaskProgressKind,
    phase: &str,
    message: impl Into<String>,
    file_index: usize,
    file_total: usize,
    file_fraction: f64,
    path: &Path,
) {
    let total = file_total.max(1);
    let percent = ((file_index as f64 + file_fraction.clamp(0.0, 1.0)) / total as f64) * 100.0;
    // 批处理进度按“文件序号 + 当前文件内部阶段”折算，图片和视频都能共享同一条进度条。
    // Batch progress is normalized as file index plus per-file stage so images and videos can share one progress bar.
    emit_progress(
        app,
        task_id,
        task,
        phase,
        message,
        (file_index + 1).min(total),
        total,
        percent,
        Some(path.display().to_string()),
    );
}

fn validate_encode_request(request: &EncodeRequest) -> Result<(), String> {
    if request.input_paths.is_empty() {
        return Err("请至少选择一个输入文件".into());
    }
    if request.output_dir.trim().is_empty() {
        return Err("请选择输出文件夹".into());
    }
    if request.text.trim().is_empty() {
        return Err("水印文本不能为空".into());
    }
    if request.text.chars().count() > 800 {
        return Err("水印文本不能超过 800 个字符".into());
    }
    if request.key_mode == KeyMode::Custom
        && request.custom_password.as_deref().unwrap_or_default().is_empty()
    {
        return Err("自定义密钥模式必须输入密码".into());
    }
    Ok(())
}

fn build_key_source(request: &DecodeRequest) -> Result<KeySource, String> {
    if let Some(path) = request.key_file.as_deref().filter(|value| !value.is_empty()) {
        let key_file = KeyFile::read(Path::new(path)).map_err(|error| error.to_string())?;
        return Ok(KeySource::KeyFile(key_file));
    }
    if let Some(password) = request.custom_password.as_deref().filter(|value| !value.is_empty()) {
        return Ok(KeySource::CustomPassword(password.to_owned()));
    }
    Err("请选择 .key 文件或输入自定义密码".into())
}

fn video_options_from_request(frame_parallelism: Option<usize>) -> VideoProcessingOptions {
    VideoProcessingOptions {
        frame_parallelism: frame_parallelism.unwrap_or(1),
    }
}
