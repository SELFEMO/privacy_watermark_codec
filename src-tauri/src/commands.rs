use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Local;
use tracing::{error, info};
use watermark_core::{
    embed_image_file_with_auto_strength, extract_image_file_with_progress, scan_image_file,
    CoreError, EmbedOptions, ExtractOptions, IntegrityStatus, KeyFile, KeyMode, KeySource,
    PrivacyScanStatus, WatermarkKey,
};

use crate::{
    cancellation::{CancellationRegistry, CancellationToken},
    evidence::{file_sha256, verify_evidence_manifest, write_evidence_manifest, EvidenceEntry},
    media::{detect_media_type, safe_stem},
    models::{
        CancelTaskRequest, DecodeItemEvent, DecodeItemResult, DecodeRequest, DecodeResponse,
        EncodeItemResult, EncodeRequest, EncodeResponse, MediaType, ProgressUnitKind, ScanItemResult,
        ScanRequest, ScanResponse, TaskOutcome, TaskProgressEvent, TaskProgressKind,
        VerifyEvidenceRequest,
    },
    progress::{
        batch_percent, clamp_percent, decode_geometry_file_fraction, emit_decode_item,
        emit_progress_detailed, emit_task_progress, ProgressDetail,
    },
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

#[tauri::command]
pub fn verify_evidence_manifest_cmd(
    request: VerifyEvidenceRequest,
) -> Result<crate::evidence::EvidenceVerifyResult, String> {
    let path = PathBuf::from(&request.manifest_path);
    let Some(key_file) = request.key_file.as_deref().filter(|value| !value.is_empty()) else {
        return Err("请提供密钥文件以核验证据".into());
    };
    let key = KeyFile::read(Path::new(key_file))
        .map_err(|error| error.to_string())?
        .to_watermark_key()
        .map_err(|error| error.to_string())?;
    let _ = request.custom_password;
    verify_evidence_manifest(&path, &key).map_err(|error| error.to_string())
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
    let mut files_succeeded = 0usize;
    let mut files_failed = 0usize;
    let video_options = video_options_from_request(request.frame_parallelism);
    for (file_index, input_string) in request.input_paths.iter().enumerate() {
        cancellation.check()?;
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            files_failed += 1;
            items.push(failed_encode_item(
                &input,
                MediaType::Image,
                format!("输入文件不存在：{}", input.display()),
            ));
            continue;
        }
        let Some(media_type) = detect_media_type(&input) else {
            files_failed += 1;
            items.push(failed_encode_item(
                &input,
                MediaType::Image,
                format!("不支持的文件类型：{}", input.display()),
            ));
            continue;
        };
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

        let encode_one = (|| -> Result<EncodeItemResult, String> {
            let original_sha256 = file_sha256(&input).map_err(|error| error.to_string())?;
            let (item_dir, key, pending_key_path) = match request.key_mode {
                KeyMode::Independent => {
                    let item_dir = output_root.join(format!("{stem}_watermarked"));
                    fs::create_dir_all(&item_dir).map_err(|error| error.to_string())?;
                    let key = WatermarkKey::random(KeyMode::Independent);
                    let key_path = item_dir.join(format!("{stem}.key"));
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
                    let report = embed_image_file_with_auto_strength(
                        &input,
                        &output,
                        &EmbedOptions {
                            text: request.text.clone(),
                            key: key.clone(),
                            strength: request.strength,
                            media_kind: "image".into(),
                            temporal: None,
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

            let written_key_path = if let Some(key_path) = &pending_key_path {
                // 独立密钥在媒体写成功（视频还要封装回读）后再落盘，失败路径不得留下能解出水印的 .key。
                // Independent keys are written only after the media succeeds (and videos pass mux round-trip) so a failed path never leaves a working .key.
                key.to_key_file()
                    .write(key_path)
                    .map_err(|error| error.to_string())?;
                Some(key_path.clone())
            } else {
                None
            };

            cancellation.check()?;
            let effective_key_path = written_key_path.as_deref().or(shared_key_path.as_deref());
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

            Ok(EncodeItemResult {
                input_path: input.display().to_string(),
                output_path: output.display().to_string(),
                key_path: written_key_path.map(|path| path.display().to_string()),
                manifest_path: manifest_path.map(|path| path.display().to_string()),
                media_type,
                psnr,
                frame_count,
                succeeded: true,
                error: None,
            })
        })();

        match encode_one {
            Ok(item) => {
                files_succeeded += 1;
                items.push(item);
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
            Err(error) => {
                error!(input = %input.display(), %error, "文件编码失败，已继续处理后续文件");
                files_failed += 1;
                items.push(failed_encode_item(&input, media_type, error));
                emit_file_progress(
                    &app,
                    task_id.clone(),
                    TaskProgressKind::Encode,
                    "completed-file",
                    format!("第 {}/{} 个文件编码失败，已继续处理后续文件", file_index + 1, total_files),
                    file_index,
                    total_files,
                    1.0,
                    &input,
                );
            }
        }
    }

    let manifest_path = if request.key_mode == KeyMode::Independent {
        None
    } else if files_succeeded > 0 {
        let path = output_root.join("batch_evidence_manifest.json");
        let signing_key = shared_key.as_ref().unwrap();
        write_evidence_manifest(&path, batch_entries, signing_key, current_release_metadata())
            .map_err(|error| error.to_string())?;
        Some(path)
    } else {
        None
    };

    if files_succeeded > 0 {
        output_root_guard.disarm();
    }

    emit_progress(
        &app,
        task_id,
        TaskProgressKind::Encode,
        if files_failed > 0 {
            "completed_with_errors"
        } else {
            "completed"
        },
        if files_failed > 0 {
            "批量编码任务结束，部分文件失败"
        } else {
            "批量编码任务完成"
        },
        total_files,
        total_files,
        100.0,
        None,
    );
    info!(completed = files_succeeded, failed = files_failed, "批量编码任务结束");
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
    let mut files_failed = 0usize;
    let mut files_succeeded = 0usize;
    for (file_index, input_string) in request.input_paths.iter().enumerate() {
        cancellation.check()?;
        let input = PathBuf::from(input_string);
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
        let item = if !input.is_file() {
            files_failed += 1;
            failed_scan_item(&input, format!("输入文件不存在：{}", input.display()))
        } else if detect_media_type(&input) != Some(MediaType::Image) {
            files_failed += 1;
            failed_scan_item(&input, format!("未知来源隐私水印扫描当前仅支持图片：{}", input.display()))
        } else {
            match scan_image_file(&input) {
                Ok(report) => {
                    files_succeeded += 1;
                    ScanItemResult {
                        input_path: input.display().to_string(),
                        status: report.status,
                        summary: report.summary,
                        detections: report.detections,
                        width: report.width,
                        height: report.height,
                        error: None,
                    }
                }
                Err(error) => {
                    error!(input = %input.display(), %error, "未知来源图片扫描失败");
                    files_failed += 1;
                    failed_scan_item(&input, error.to_string())
                }
            }
        };
        items.push(item);
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
        if files_failed > 0 {
            "completed_with_errors"
        } else {
            "completed"
        },
        if files_failed > 0 {
            "未知来源图片扫描结束，部分文件失败"
        } else {
            "未知来源图片扫描完成"
        },
        total_files,
        total_files,
        100.0,
        None,
    );
    info!(completed = files_succeeded, failed = files_failed, "未知来源图片隐私水印扫描完成");
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
    let mut files_succeeded = 0usize;
    let mut files_failed = 0usize;
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

        let item_result: Result<DecodeItemResult, String> = match media_type {
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
                extract_image_file_with_progress(
                    &input,
                    &key_source,
                    ExtractOptions::default(),
                    |search| {
                        cancellation.check().map_err(|_| CoreError::Cancelled)?;
                        // 全图插值中的静默检查只负责让取消及时生效，避免每 64 行向前端发送重复进度事件造成新的界面负担。
                        // Silent checkpoints during full-image interpolation exist only for responsive cancellation, avoiding duplicate frontend events every 64 rows that would create new UI overhead.
                        if !search.should_emit {
                            return Ok(());
                        }
                        let file_fraction = decode_geometry_file_fraction(
                            search.stage,
                            search.current,
                            search.total,
                            search.sub_progress,
                        );
                        let percent = batch_percent(file_index, total_files, file_fraction);
                        let is_verify = search.stage == "verify";
                        let stage_label = match search.stage {
                            "identity" => "原始方向快速检测",
                            "resize-priority" => "常见缩放快速检测",
                            "rotation-padding-hint" => "白边角度预估",
                            "rotation-padding-forced" => "白边提示强制验证",
                            "rotation-fill-forced" => "无白边铺满裁切强制验证",
                            "rotation-padding-angle" => "白边角度精化",
                            "rotation-padding-scale" => "白边缩放联合校正",
                            "rotation-padding-fine" => "白边高精度校正",
                            "rotation-fixed-priority" => "固定画布旋转检测",
                            "rotation-fit-priority" => "扩展画布旋转检测",
                            "rotation-tight-priority" => "裁边旋转检测",
                            "rotation-priority" => "常见旋转快速检测",
                            "rotation-layout-near" => "近角度布局搜索",
                            "rotation-layout-far" => "远角度布局搜索",
                            "rotation-layout-fallback" => "备用画布布局搜索",
                            "rotation-resize-priority" => "旋转缩放快速精化",
                            "rotation-resize-refine" => "旋转缩放联合精化",
                            "rotation-resize-fine" => "旋转缩放高精度校正",
                            "rotation-coarse" => "旋转粗搜索",
                            "rotation-refine" => "旋转精细搜索",
                            "rotation-fine" => "旋转高精度搜索",
                            "resize-coarse" => "缩放粗搜索",
                            "resize-refine" => "缩放精细搜索",
                            "resize-fine" => "缩放高精度搜索",
                            "orientation" => "直角方向兜底检测",
                            "rotation-range-narrow" => "旋转范围归一",
                            "combined" => "旋转与缩放联合验证",
                            "verify" => "完整校正图验证",
                            _ => "图片几何配准",
                        };
                        let worker_hint = if search.workers > 1 {
                            format!("，CPU {} 线程", search.workers)
                        } else {
                            String::new()
                        };
                        let message = if is_verify {
                            format!(
                                "评分已结束，正在完整验证第 {}/{} 张校正图，旋转 {:+.2}°，缩放 ×{:.3}",
                                search.current,
                                search.total,
                                search.rotation_degrees,
                                search.scale
                            )
                        } else {
                            format!(
                                "正在执行{}：候选 {}/{}，旋转 {:+.2}°，缩放 ×{:.3}{}",
                                stage_label,
                                search.current,
                                search.total,
                                search.rotation_degrees,
                                search.scale,
                                worker_hint
                            )
                        };
                        // 阶段名比单一候选总数更能解释分层搜索正在做什么，线程提示也让并行评分与真正停滞可以被明确区分。
                        // Stage names explain layered search better than a single candidate counter, while the worker hint distinguishes parallel scoring from a genuine stall.
                        emit_progress_detailed(
                            &app,
                            task_id.clone(),
                            TaskProgressKind::Decode,
                            if is_verify {
                                "verifying-image"
                            } else {
                                "registering-image"
                            },
                            message,
                            ProgressDetail {
                                current: (file_index + 1).min(total_files.max(1)),
                                total: total_files.max(1),
                                percent,
                                current_path: Some(input.display().to_string()),
                                outcome: TaskOutcome::Running,
                                file_fraction: Some(file_fraction),
                                files_succeeded: Some(files_succeeded),
                                files_failed: Some(files_failed),
                                unit_kind: Some(ProgressUnitKind::Geometry),
                                unit_current: Some(search.current),
                                unit_total: Some(search.total),
                                unit_label: Some(format!(
                                    "{stage_label} {:+.2}° ×{:.3}",
                                    search.rotation_degrees, search.scale
                                )),
                            },
                        );
                        Ok(())
                    },
                )
                .map(|report| DecodeItemResult {
                    input_path: input.display().to_string(),
                    succeeded: true,
                    error: None,
                    media_type,
                    text: report.text,
                    integrity: report.integrity,
                    fingerprint_distance: Some(report.fingerprint_distance),
                    corrected_codewords: report.corrected_codewords,
                    frame_count: None,
                    expected_frame_count: None,
                    valid_frames: None,
                    modified_frames: None,
                    missing_frames: None,
                    inserted_frames: None,
                    deleted_frames: None,
                    duplicate_frames: None,
                    out_of_order_frames: None,
                    replaced_frames: None,
                    unreadable_frames: None,
                    transformed_frames: None,
                    timeline_anomalies: Vec::new(),
                    tamper_regions: report.tamper_regions,
                    sync_registration: Some(report.sync_registration),
                })
                .map_err(|error| {
                    error!(input = %input.display(), %error, "图片解码失败");
                    error.to_string()
                })
            }
            MediaType::Video => decode_video(
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
            )
            .map(|report| DecodeItemResult {
                input_path: input.display().to_string(),
                succeeded: true,
                error: None,
                media_type,
                text: report.text,
                integrity: report.integrity,
                fingerprint_distance: None,
                corrected_codewords: report.corrected_codewords,
                frame_count: Some(report.frame_count),
                expected_frame_count: report.expected_frame_count,
                valid_frames: Some(report.valid_frames),
                modified_frames: Some(report.modified_frames),
                missing_frames: Some(report.missing_frames),
                inserted_frames: Some(report.inserted_frames),
                deleted_frames: Some(report.deleted_frames),
                duplicate_frames: Some(report.duplicate_frames),
                out_of_order_frames: Some(report.out_of_order_frames),
                replaced_frames: Some(report.replaced_frames),
                unreadable_frames: Some(report.unreadable_frames),
                transformed_frames: Some(report.transformed_frames),
                timeline_anomalies: report.timeline_anomalies,
                tamper_regions: report.tamper_regions,
                sync_registration: report.sync_registration,
            })
            .map_err(|error| {
                error!(input = %input.display(), %error, "视频解码失败");
                error
            }),
        };

        // 单个文件的水印损坏、密钥不匹配或格式异常不应中止整批任务；保留失败条目后继续，用户才能看到每一张图片的独立结论。
        // A damaged watermark, mismatched key, or malformed file must not abort the whole batch; retaining a failed item and continuing gives every image an independent outcome.
        let succeeded = item_result.is_ok();
        let result = item_result.unwrap_or_else(|error| {
            failed_decode_item(&input, media_type, error)
        });
        // 结果事件在加入最终集合的同一位置发送，是为了保证前端流式顺序与命令最终返回顺序完全一致，避免快速任务出现重复或跳项。
        // The item event is emitted at the same point it enters the final collection so frontend streaming order exactly matches the command's final response, avoiding duplicates or gaps in fast batches.
        emit_decode_item(
            &app,
            DecodeItemEvent {
                task_id: task_id.clone(),
                current: file_index + 1,
                total: total_files,
                item: result.clone(),
            },
        );
        items.push(result);
        if succeeded {
            files_succeeded += 1;
        } else {
            files_failed += 1;
        }
        // 成功或 HeaderNotFound 都必须立刻把当前文件打到 100%，否则评分封顶后的验证阶段会让界面一直停在 90%–98%。
        // Success and HeaderNotFound both jump the current file to 100% immediately, otherwise the post-scoring verify phase would leave the UI parked between 90% and 98%.
        let mut done = TaskProgressEvent::running(
            task_id.clone(),
            TaskProgressKind::Decode,
            "completed-file",
            if succeeded {
                format!("第 {}/{} 个文件解码检测完成", file_index + 1, total_files)
            } else {
                format!("第 {}/{} 个文件检测失败，已继续处理后续文件", file_index + 1, total_files)
            },
            (file_index + 1).min(total_files.max(1)),
            total_files.max(1),
            batch_percent(file_index, total_files, 1.0),
            Some(input.display().to_string()),
        );
        done.file_fraction = Some(1.0);
        done.files_succeeded = Some(files_succeeded);
        done.files_failed = Some(files_failed);
        done.unit_kind = Some(ProgressUnitKind::File);
        emit_task_progress(&app, done);
        // 已完成文件先发送结果再检查取消，是为了让用户在按下取消的瞬间仍保留刚刚完成的有效结论，而不会因任务终止丢失该条目。
        // A completed file is emitted before the cancellation check so a valid result that finishes as the user cancels is retained instead of being discarded with the task termination.
        cancellation.check()?;
    }

    let mut finished = TaskProgressEvent::running(
        task_id,
        TaskProgressKind::Decode,
        if files_failed > 0 {
            "completed_with_errors"
        } else {
            "completed"
        },
        if files_failed > 0 {
            "批量解码检测任务结束，部分文件失败"
        } else {
            "批量解码检测任务完成"
        },
        total_files,
        total_files,
        100.0,
        None,
    );
    finished.outcome = Some(if files_failed > 0 {
        TaskOutcome::CompletedWithErrors
    } else {
        TaskOutcome::Completed
    });
    finished.files_succeeded = Some(files_succeeded);
    finished.files_failed = Some(files_failed);
    finished.unit_kind = Some(ProgressUnitKind::File);
    emit_task_progress(&app, finished);
    let succeeded = files_succeeded;
    info!(completed = items.len(), succeeded, "批量解码任务完成");
    Ok(DecodeResponse { items })
}

fn failed_encode_item(input: &Path, media_type: MediaType, error: String) -> EncodeItemResult {
    EncodeItemResult {
        input_path: input.display().to_string(),
        output_path: String::new(),
        key_path: None,
        manifest_path: None,
        media_type,
        psnr: None,
        frame_count: None,
        succeeded: false,
        error: Some(error),
    }
}

fn failed_scan_item(input: &Path, error: String) -> ScanItemResult {
    ScanItemResult {
        input_path: input.display().to_string(),
        status: PrivacyScanStatus::NotDetected,
        summary: error.clone(),
        detections: Vec::new(),
        width: None,
        height: None,
        error: Some(error),
    }
}

fn failed_decode_item(input: &Path, media_type: MediaType, error: String) -> DecodeItemResult {
    DecodeItemResult {
        input_path: input.display().to_string(),
        succeeded: false,
        error: Some(error),
        media_type,
        text: String::new(),
        integrity: IntegrityStatus::Uncertain,
        fingerprint_distance: None,
        corrected_codewords: 0,
        frame_count: None,
        expected_frame_count: None,
        valid_frames: None,
        modified_frames: None,
        missing_frames: None,
        inserted_frames: None,
        deleted_frames: None,
        duplicate_frames: None,
        out_of_order_frames: None,
        replaced_frames: None,
        unreadable_frames: None,
        transformed_frames: None,
        timeline_anomalies: Vec::new(),
        tamper_regions: Vec::new(),
        sync_registration: None,
    }
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
    let mut event = TaskProgressEvent::running(
        task_id,
        task,
        phase,
        message,
        current,
        total,
        clamp_percent(percent),
        current_path,
    );
    event.outcome = Some(match phase {
        "completed" => TaskOutcome::Completed,
        "completed_with_errors" => TaskOutcome::CompletedWithErrors,
        "failed" => TaskOutcome::Failed,
        "cancelled" => TaskOutcome::Cancelled,
        _ => TaskOutcome::Running,
    });
    emit_task_progress(app, event);
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
    let mut event = TaskProgressEvent::running(
        task_id,
        task,
        phase,
        message,
        (file_index + 1).min(total),
        total,
        batch_percent(file_index, file_total, file_fraction),
        Some(path.display().to_string()),
    );
    event.file_fraction = Some(file_fraction.clamp(0.0, 1.0));
    event.unit_kind = Some(ProgressUnitKind::File);
    // 批处理进度按“文件序号 + 当前文件内部阶段”折算，图片和视频都能共享同一条进度条。
    // Batch progress is normalized as file index plus per-file stage so images and videos can share one progress bar.
    emit_task_progress(app, event);
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
    if request.key_mode == KeyMode::Custom {
        let password = request.custom_password.as_deref().unwrap_or_default();
        if password.chars().count() < 12 {
            return Err("自定义密码至少需要 12 个字符".into());
        }
    }
    Ok(())
}

fn build_key_source(request: &DecodeRequest) -> Result<KeySource, String> {
    let mut paths = request.key_files.clone().unwrap_or_default();
    if let Some(path) = request.key_file.as_deref().filter(|value| !value.is_empty()) {
        paths.push(path.to_owned());
    }
    paths.retain(|path| !path.trim().is_empty());
    paths.sort();
    paths.dedup();

    if !paths.is_empty() {
        let mut key_files = Vec::with_capacity(paths.len());
        for path in paths {
            key_files.push(KeyFile::read(Path::new(&path)).map_err(|error| {
                format!("读取密钥文件失败（{path}）：{error}")
            })?);
        }
        // 多个独立密钥一次载入后由核心按公开头 salt 自动匹配，既支持乱序批量导入，也避免为每张图片重复扫描所有密钥文件。
        // Loading independent keys once and matching them by public-header salt supports unordered batch imports and avoids rereading every key file for every image.
        return Ok(if key_files.len() == 1 {
            KeySource::KeyFile(key_files.remove(0))
        } else {
            KeySource::KeyFiles(key_files)
        });
    }
    if let Some(password) = request.custom_password.as_deref().filter(|value| !value.is_empty()) {
        return Ok(KeySource::CustomPassword(password.to_owned()));
    }
    Err("请选择一个或多个 .key 文件，或输入自定义密码".into())
}

fn video_options_from_request(frame_parallelism: Option<usize>) -> VideoProcessingOptions {
    VideoProcessingOptions {
        frame_parallelism: frame_parallelism.unwrap_or(1),
    }
}
