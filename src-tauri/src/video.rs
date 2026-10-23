use std::{
    collections::HashMap,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use tracing::info;
use walkdir::WalkDir;
use watermark_core::{
    crypto::random_bytes, embed_image_file_with_auto_strength, extract_image_file_with_recovery,
    CoreError, EmbedOptions, ExtractOptions, IntegrityStatus, KeySource, RecoveryMode,
    RecoveryOptions, SyncRegistration, TamperRegion, TemporalContext, WatermarkKey,
};

use crate::{
    cancellation::CancellationToken,
    ffmpeg,
    models::{
        ProgressUnitKind, TaskOutcome, TaskProgressKind, TimelineAnomalyKind, TimelineAnomalyRange,
    },
    progress::{batch_percent, emit_progress_detailed, ProgressDetail},
    storage,
};
use tauri::AppHandle;

const MAX_VIDEO_PIXEL_FRAMES: u64 = 1920 * 1080 * 8_000;

pub fn effective_video_strength(ui: f32) -> f32 {
    ui.clamp(3.0, 20.0)
}

pub(crate) fn video_frame_failure_aborts_clip(failed_frames: usize, total_frames: usize) -> bool {
    // 只按整段失败率中止；连续暗场贴线帧不应在失败率仍低于 10% 时毁掉整段。
    // Abort only on clip-wide failure rate; a streak of dark near-miss frames must not kill the clip while the rate is still under 10%.
    let total = total_frames.max(1) as f64;
    (failed_frames as f64 / total) > 0.10
}

pub(crate) fn ffmpeg_input_arg(path: &Path) -> OsString {
    let file_name = path.file_name().unwrap_or_default();
    if !file_name.to_string_lossy().starts_with('-') {
        return path.as_os_str().to_owned();
    }
    if let Ok(canonical) = dunce_canonicalize(path) {
        #[cfg(windows)]
        {
            let text = canonical.to_string_lossy();
            if text.starts_with(r"\\?\") {
                return canonical.into_os_string();
            }
            let mut prefixed = OsString::from(r"\\?\");
            prefixed.push(canonical.as_os_str());
            return prefixed;
        }
        #[cfg(not(windows))]
        {
            return canonical.into_os_string();
        }
    }
    let mut prefixed = OsString::from(".");
    prefixed.push(std::path::MAIN_SEPARATOR.to_string());
    prefixed.push(file_name);
    prefixed
}

fn dunce_canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    path.canonicalize()
}

fn video_pixel_frames(width: u32, height: u32, frame_count: usize) -> u64 {
    u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(frame_count as u64)
}

pub(crate) fn exceeds_max_video_pixel_frames(width: u32, height: u32, frame_count: usize) -> bool {
    video_pixel_frames(width, height, frame_count) > MAX_VIDEO_PIXEL_FRAMES
}

#[derive(Debug, Clone, Copy)]
pub struct VideoProcessingOptions {
    pub frame_parallelism: usize,
}

#[derive(Debug, Clone)]
pub struct VideoProgressContext {
    pub task_id: Option<String>,
    pub task: TaskProgressKind,
    pub file_index: usize,
    pub file_total: usize,
    pub file_path: String,
}

impl VideoProgressContext {
    fn emit(
        &self,
        app: &AppHandle,
        phase: &str,
        message: impl Into<String>,
        file_fraction: f64,
        current: usize,
        total: usize,
    ) {
        self.emit_units(
            app,
            phase,
            message,
            file_fraction,
            ProgressUnitKind::Frame,
            current,
            total,
            None,
        );
    }

    fn emit_units(
        &self,
        app: &AppHandle,
        phase: &str,
        message: impl Into<String>,
        file_fraction: f64,
        unit_kind: ProgressUnitKind,
        unit_current: usize,
        unit_total: usize,
        unit_label: Option<String>,
    ) {
        let total_files = self.file_total.max(1);
        // current/total 只报文件位置；帧或几何候选必须走 unit*，否则 UI 会把 512/1025 当成文件数。
        // current/total stay on the file position; frame or geometry candidates must use unit* so the UI cannot treat 512/1025 as a file count.
        emit_progress_detailed(
            app,
            self.task_id.clone(),
            self.task,
            phase,
            message,
            ProgressDetail {
                current: (self.file_index + 1).min(total_files),
                total: total_files,
                percent: batch_percent(self.file_index, self.file_total, file_fraction),
                current_path: Some(self.file_path.clone()),
                outcome: TaskOutcome::Running,
                file_fraction: Some(file_fraction.clamp(0.0, 1.0)),
                files_succeeded: None,
                files_failed: None,
                unit_kind: Some(unit_kind),
                unit_current: Some(unit_current),
                unit_total: Some(unit_total),
                unit_label,
            },
        );
    }
}

#[derive(Debug, Clone)]
pub struct VideoEncodeReport {
    pub frame_count: usize,
}

#[derive(Debug, Clone)]
pub struct VideoDecodeReport {
    pub text: String,
    pub integrity: IntegrityStatus,
    pub corrected_codewords: usize,
    pub frame_count: usize,
    pub expected_frame_count: Option<usize>,
    pub valid_frames: usize,
    pub modified_frames: usize,
    pub missing_frames: usize,
    pub inserted_frames: usize,
    pub deleted_frames: usize,
    pub duplicate_frames: usize,
    pub out_of_order_frames: usize,
    pub replaced_frames: usize,
    pub unreadable_frames: usize,
    pub transformed_frames: usize,
    pub timeline_anomalies: Vec<TimelineAnomalyRange>,
    pub tamper_regions: Vec<TamperRegion>,
    pub sync_registration: Option<SyncRegistration>,
}

pub fn encode_video(
    app: &AppHandle,
    input: &Path,
    output: &Path,
    text: &str,
    key: &WatermarkKey,
    strength: f32,
    options: VideoProcessingOptions,
    cancellation: &CancellationToken,
    progress: VideoProgressContext,
) -> Result<VideoEncodeReport, String> {
    let tools = ffmpeg::bundled_tools(app)?;
    let work_dir = storage::create_work_dir("video_encode").map_err(|error| error.to_string())?;
    let source_frames = work_dir.path().join("source");
    let marked_frames = work_dir.path().join("marked");
    fs::create_dir_all(&source_frames).map_err(|error| error.to_string())?;
    fs::create_dir_all(&marked_frames).map_err(|error| error.to_string())?;

    info!(input = %input.display(), "开始使用 FFmpeg 解码视频帧");
    let input_duration_seconds = probe_media_duration_seconds(&tools.ffprobe, input)
        .ok()
        .flatten();
    progress.emit(
        app,
        "extracting-video-frames",
        "正在使用 FFmpeg 抽取视频帧",
        0.04,
        0,
        0,
    );
    run_ffmpeg_with_progress(
        app,
        &progress,
        cancellation,
        "extracting-video-frames",
        "FFmpeg 正在抽取视频帧，输出会先写入临时工作区",
        0.04,
        0.28,
        0,
        0,
        Some(source_frames.clone()),
        input_duration_seconds,
        &tools.ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("warning"),
            OsString::from("-nostats"),
            OsString::from("-progress"),
            OsString::from("pipe:1"),
            OsString::from("-y"),
            OsString::from("-i"),
            ffmpeg_input_arg(input),
            OsString::from("-vsync"),
            OsString::from("0"),
            source_frames.join("frame_%09d.png").into_os_string(),
        ],
    )?;

    cancellation.check()?;
    let frames = sorted_png_files(&source_frames);
    if frames.is_empty() {
        return Err("FFmpeg 未能从视频中解码出任何画面帧".into());
    }
    let first_png = &frames[0];
    let (frame_width, frame_height) = probe_png_size(first_png)?;
    if exceeds_max_video_pixel_frames(frame_width, frame_height, frames.len()) {
        return Err(format!(
            "视频像素帧预算超限：{}×{}×{} 超过允许的 {}，请缩短视频或降低分辨率",
            frame_width,
            frame_height,
            frames.len(),
            MAX_VIDEO_PIXEL_FRAMES
        ));
    }
    progress.emit(
        app,
        "processing-video-frames",
        "视频帧已抽取，开始逐帧嵌入水印",
        0.30,
        0,
        frames.len(),
    );

    let parallelism = normalize_parallelism(options.frame_parallelism);
    let clip_id = random_bytes::<16>();
    let frame_count = u32::try_from(frames.len()).map_err(|_| "视频帧数超出协议范围".to_owned())?;
    let video_strength = effective_video_strength(strength);
    let mut failed_frames = 0usize;
    let mut consecutive_failures = 0usize;
    for (chunk_index, chunk) in frames.chunks(parallelism).enumerate() {
        cancellation.check()?;
        let start_index = chunk_index * parallelism;
        let chunk_results = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(chunk.len());
            for (offset, frame) in chunk.iter().enumerate() {
                let frame = frame.clone();
                let frame_index = u32::try_from(start_index + offset)
                    .map_err(|_| "视频帧序号超出协议范围".to_owned())?;
                let output_frame = marked_frames.join(frame.file_name().unwrap());
                let key = key.clone();
                let text = text.to_owned();
                handles.push(scope.spawn(move || {
                    let options = EmbedOptions {
                        text,
                        key,
                        // 视频强度只按用户滑条钳到 3–20，不再偷偷抬到 16；PSNR 不足时由核心逐级降到 4。
                        // Video strength follows the UI slider clamped to 3–20 and is no longer silently raised to 16; the core backs off to 4 when PSNR is too low.
                        strength: video_strength,
                        media_kind: "video_frame".into(),
                        temporal: Some(TemporalContext {
                            clip_id,
                            frame_index,
                            frame_count,
                        }),
                    };
                    match embed_image_file_with_auto_strength(&frame, &output_frame, &options) {
                        Ok(_) => Ok(()),
                        Err(error) => {
                            // 单帧失败时复制原帧以保住封装序列，直到失败率或连续失败触发整段中止。
                            // A failed frame is copied unmarked so the mux sequence stays intact until the failure rate or streak aborts the clip.
                            fs::copy(&frame, &output_frame)
                                .map_err(|copy_error| copy_error.to_string())?;
                            Err(error.to_string())
                        }
                    }
                }));
            }

            let mut results = Vec::with_capacity(handles.len());
            for handle in handles {
                results.push(
                    handle
                        .join()
                        .map_err(|_| "视频帧编码线程异常终止".to_owned())?,
                );
            }
            Ok::<Vec<Result<(), String>>, String>(results)
        })?;
        for result in chunk_results {
            match result {
                Ok(()) => consecutive_failures = 0,
                Err(error) => {
                    failed_frames += 1;
                    consecutive_failures += 1;
                    if video_frame_failure_aborts_clip(failed_frames, frames.len()) {
                        return Err(format!(
                            "视频逐帧嵌入失败过多（失败 {failed_frames}/{}，连续 {consecutive_failures}）。{error}",
                            frames.len()
                        ));
                    }
                }
            }
        }
        cancellation.check()?;

        let processed = ((chunk_index + 1) * parallelism).min(frames.len());
        let frame_fraction = processed as f64 / frames.len() as f64;
        progress.emit(
            app,
            "processing-video-frames",
            format!("正在嵌入视频帧水印：{processed}/{}", frames.len()),
            0.30 + frame_fraction * 0.54,
            processed,
            frames.len(),
        );
        info!(
            processed,
            total = frames.len(),
            parallelism,
            "视频帧水印处理进度"
        );
    }

    cancellation.check()?;
    let frame_rate = probe_frame_rate(&tools.ffprobe, input)?;
    info!(frame_rate = %frame_rate, frames = frames.len(), "开始重新编码视频并复制音轨");
    // 使用 H.264 lossless + yuv444p 是为了尽量保留亮度频域系数，否则本软件刚生成的视频也可能在再次解码时丢失水印。
    // H.264 lossless with yuv444p preserves luminance-frequency coefficients as much as possible; otherwise even videos generated by this app may lose the watermark during decoding.
    progress.emit(
        app,
        "muxing-video",
        "正在重新封装视频并复制音轨",
        0.86,
        frames.len(),
        frames.len(),
    );
    run_ffmpeg_with_progress(
        app,
        &progress,
        cancellation,
        "muxing-video",
        "FFmpeg 正在重新封装输出视频",
        0.86,
        0.98,
        frames.len(),
        frames.len(),
        None,
        input_duration_seconds,
        &tools.ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("warning"),
            OsString::from("-nostats"),
            OsString::from("-progress"),
            OsString::from("pipe:1"),
            OsString::from("-y"),
            OsString::from("-framerate"),
            OsString::from(frame_rate),
            OsString::from("-i"),
            marked_frames.join("frame_%09d.png").into_os_string(),
            OsString::from("-i"),
            ffmpeg_input_arg(input),
            OsString::from("-map"),
            OsString::from("0:v:0"),
            OsString::from("-map"),
            OsString::from("1:a?"),
            OsString::from("-c:v"),
            OsString::from("libx264"),
            OsString::from("-preset"),
            OsString::from("medium"),
            OsString::from("-crf"),
            OsString::from("0"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv444p"),
            OsString::from("-c:a"),
            OsString::from("copy"),
            OsString::from("-shortest"),
            output.as_os_str().to_owned(),
        ],
    )?;

    verify_watermarked_video_frames(
        app,
        &tools.ffmpeg,
        output,
        key,
        frames.len(),
        &progress,
        cancellation,
    )?;

    progress.emit(
        app,
        "completed-file",
        "当前视频编码完成",
        1.0,
        frames.len(),
        frames.len(),
    );
    info!(output = %output.display(), frames = frames.len(), "视频水印编码完成");
    Ok(VideoEncodeReport {
        frame_count: frames.len(),
    })
}

pub fn decode_video(
    app: &AppHandle,
    input: &Path,
    key_source: &KeySource,
    options: VideoProcessingOptions,
    recovery_mode: RecoveryMode,
    cancellation: &CancellationToken,
    progress: VideoProgressContext,
) -> Result<VideoDecodeReport, String> {
    let tools = ffmpeg::bundled_tools(app)?;
    let work_dir = storage::create_work_dir("video_decode").map_err(|error| error.to_string())?;
    let frames_dir = work_dir.path().join("frames");
    fs::create_dir_all(&frames_dir).map_err(|error| error.to_string())?;

    let input_duration_seconds = probe_media_duration_seconds(&tools.ffprobe, input)
        .ok()
        .flatten();
    progress.emit(
        app,
        "extracting-video-frames",
        "正在使用 FFmpeg 抽取待检测视频帧",
        0.04,
        0,
        0,
    );
    run_ffmpeg_with_progress(
        app,
        &progress,
        cancellation,
        "extracting-video-frames",
        "FFmpeg 正在抽取待检测视频帧，输出会先写入临时工作区",
        0.04,
        0.28,
        0,
        0,
        Some(frames_dir.clone()),
        input_duration_seconds,
        &tools.ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("warning"),
            OsString::from("-nostats"),
            OsString::from("-progress"),
            OsString::from("pipe:1"),
            OsString::from("-y"),
            OsString::from("-i"),
            ffmpeg_input_arg(input),
            OsString::from("-vsync"),
            OsString::from("0"),
            frames_dir.join("frame_%09d.png").into_os_string(),
        ],
    )?;

    cancellation.check()?;
    let frames = sorted_png_files(&frames_dir);
    if frames.is_empty() {
        return Err("FFmpeg 未能从视频中解码出任何画面帧".into());
    }
    progress.emit(
        app,
        "checking-video-frames",
        "视频帧已抽取，开始逐帧解码检测",
        0.30,
        0,
        frames.len(),
    );

    let parallelism = normalize_parallelism(options.frame_parallelism);
    let mut reports = Vec::with_capacity(frames.len());
    for (chunk_index, chunk) in frames.chunks(parallelism).enumerate() {
        cancellation.check()?;
        let start_index = chunk_index * parallelism;
        let mut chunk_reports = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(chunk.len());
            for (offset, frame) in chunk.iter().enumerate() {
                let frame = frame.clone();
                let key_source = key_source.clone();
                let cancellation = cancellation.callback();
                handles.push(scope.spawn(move || {
                    let physical_index = start_index + offset;
                    let options = ExtractOptions {
                        // 首轮并行解码关闭几何搜索，是为了快速覆盖正常帧；只有失败帧才进入串行重配准，避免攻击者用单帧拖垮整段视频。
                        // The first parallel pass disables geometry search to cover normal frames quickly; only failures enter serial re-registration so one attacked frame cannot stall the whole video.
                        allow_registration: false,
                    };
                    (
                        physical_index,
                        extract_image_file_with_recovery(
                            &frame,
                            &key_source,
                            options,
                            RecoveryOptions {
                                mode: recovery_mode,
                                cancellation: Some(cancellation),
                                max_duration: None,
                            },
                            |_| Ok(()),
                        ),
                    )
                }));
            }
            let mut completed = Vec::with_capacity(chunk.len());
            for handle in handles {
                completed.push(
                    handle
                        .join()
                        .map_err(|_| "视频帧解码线程异常终止".to_owned())?,
                );
            }
            Ok::<Vec<_>, String>(completed)
        })?;
        reports.append(&mut chunk_reports);
        let processed = ((chunk_index + 1) * parallelism).min(frames.len());
        let frame_fraction = processed as f64 / frames.len() as f64;
        progress.emit(
            app,
            "checking-video-frames",
            format!("正在逐帧解码检测：{processed}/{}", frames.len()),
            0.30 + frame_fraction * 0.56,
            processed,
            frames.len(),
        );
    }
    reports.sort_by_key(|(index, _)| *index);

    let failed_total = reports.iter().filter(|(_, result)| result.is_err()).count();
    let mut failed_processed = 0_usize;
    let recovery_deadline = Instant::now()
        + Duration::from_secs(match recovery_mode {
            RecoveryMode::Standard => 10,
            RecoveryMode::Deep => 60,
        });
    let mut recovery_timed_out = false;
    for (physical_index, result) in &mut reports {
        if matches!(
            result,
            Err(CoreError::HeaderNotFound
                | CoreError::PayloadCorrupted
                | CoreError::RecoveryTimeout)
        ) {
            cancellation.check()?;
            // 失败帧共享整段恢复预算，避免一段无水印视频把单帧上限累乘成数小时等待。
            // Failed frames share one clip recovery budget so an unmarked video cannot multiply a per-frame limit into hours of waiting.
            if Instant::now() >= recovery_deadline {
                recovery_timed_out = true;
                break;
            }
            let failed_before = failed_processed;
            let frame_number = *physical_index + 1;
            *result = extract_image_file_with_recovery(
                &frames[*physical_index],
                key_source,
                ExtractOptions {
                    allow_registration: true,
                },
                RecoveryOptions {
                    mode: recovery_mode,
                    cancellation: Some(cancellation.callback()),
                    max_duration: Some(recovery_deadline.saturating_duration_since(Instant::now())),
                },
                |search| {
                    cancellation.check().map_err(|_| CoreError::Cancelled)?;
                    if !search.should_emit {
                        return Ok(());
                    }
                    let candidate_fraction = if search.stage == "verify" {
                        let total = search.total.max(1) as f64;
                        (search.current.saturating_sub(1) as f64
                            + f64::from(search.sub_progress.clamp(0.0, 1.0)))
                            / total
                    } else {
                        search.current as f64 / search.total.max(1) as f64
                    };
                    // 评分只占该失败帧回退区间的前 80%，完整验证占后 20%，避免 1025/1025 之后验证阶段把进度条往回拨。
                    // Scoring uses the first 80% of this failed frame's fallback slice and full verification uses the last 20%, so the bar cannot rewind after 1025/1025.
                    let within_frame = if search.stage == "verify" {
                        0.80 + candidate_fraction * 0.20
                    } else {
                        candidate_fraction * 0.80
                    };
                    let fallback_fraction =
                        (failed_before as f64 + within_frame) / failed_total.max(1) as f64;
                    let message = if search.stage == "verify" {
                        format!(
                            "评分已结束，正在完整验证第 {} 帧第 {}/{} 张校正图，旋转 {:+.2}°，缩放 ×{:.3}",
                            frame_number,
                            search.current,
                            search.total,
                            search.rotation_degrees,
                            search.scale
                        )
                    } else {
                        format!(
                            "正在恢复第 {frame_number} 帧的几何同步：候选 {}/{}，旋转 {:+.2}°，缩放 ×{:.3}",
                            search.current,
                            search.total,
                            search.rotation_degrees,
                            search.scale
                        )
                    };
                    progress.emit_units(
                        app,
                        "registering-video-frames",
                        message,
                        0.86 + fallback_fraction * 0.07,
                        ProgressUnitKind::Geometry,
                        search.current,
                        search.total,
                        Some(format!(
                            "{:+.2}° ×{:.3}",
                            search.rotation_degrees, search.scale
                        )),
                    );
                    Ok(())
                },
            );
            cancellation.check()?;
            recovery_timed_out |= matches!(result, Err(CoreError::RecoveryTimeout))
                || Instant::now() >= recovery_deadline;
            failed_processed += 1;
        }
    }
    progress.emit(
        app,
        "summarizing-video",
        "正在执行时间轴序列对齐并汇总检测结果",
        0.94,
        frames.len(),
        frames.len(),
    );

    let mut temporal_votes: HashMap<(String, String, u32), usize> = HashMap::new();
    let mut legacy_votes: HashMap<String, usize> = HashMap::new();
    for (_, result) in &reports {
        if let Ok(report) = result {
            if let Some(temporal) = &report.temporal {
                *temporal_votes
                    .entry((
                        report.text.clone(),
                        temporal.clip_id_hex.clone(),
                        temporal.frame_count,
                    ))
                    .or_default() += 1;
            } else {
                *legacy_votes.entry(report.text.clone()).or_default() += 1;
            }
        }
    }
    let expected_temporal = temporal_votes
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(identity, _)| identity);
    let expected_legacy_text = legacy_votes
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(text, _)| text);
    if expected_temporal.is_none() && expected_legacy_text.is_none() {
        if recovery_timed_out {
            return Err(CoreError::RecoveryTimeout.to_string());
        }
        return Err(format!(
            "视频逐帧校验失败：共 {} 帧，没有任何帧成功提取水印",
            frames.len()
        ));
    }

    let expected_frame_count = expected_temporal
        .as_ref()
        .map(|(_, _, count)| *count as usize);
    let mut temporal_observations = vec![None; reports.len()];
    let mut legacy_valid_frames = 0_usize;
    let mut modified_frames = 0_usize;
    let mut transformed_frames = 0_usize;
    let mut corrected_codewords = 0_usize;
    let mut tamper_regions = Vec::new();
    let mut sync_registration = None;

    for (physical_index, result) in &reports {
        cancellation.check()?;
        let Ok(report) = result else {
            continue;
        };
        let identity_matches = if let Some((text, clip_id, frame_count)) = &expected_temporal {
            report.text.as_str() == text.as_str()
                && report.temporal.as_ref().is_some_and(|temporal| {
                    temporal.clip_id_hex.as_str() == clip_id.as_str()
                        && temporal.frame_count == *frame_count
                })
        } else {
            expected_legacy_text
                .as_ref()
                .is_some_and(|text| report.text.as_str() == text.as_str())
                && report.temporal.is_none()
        };
        if !identity_matches {
            continue;
        }

        if let Some(expected_count) = expected_frame_count {
            let Some(temporal) = &report.temporal else {
                continue;
            };
            let expected_index = temporal.frame_index as usize;
            if expected_index >= expected_count {
                continue;
            }
            temporal_observations[*physical_index] = Some(expected_index);
        } else {
            legacy_valid_frames += 1;
        }

        corrected_codewords += report.corrected_codewords;
        if report.integrity != IntegrityStatus::Intact {
            modified_frames += 1;
        }
        if !report.sync_registration.is_identity() {
            transformed_frames += 1;
            sync_registration.get_or_insert(report.sync_registration);
        } else if sync_registration.is_none() {
            sync_registration = Some(report.sync_registration);
        }
        tamper_regions.extend(report.tamper_regions.iter().cloned());
    }

    let (
        valid_frames,
        missing_frames,
        inserted_frames,
        deleted_frames,
        duplicate_frames,
        out_of_order_frames,
        replaced_frames,
        unreadable_frames,
        timeline_anomalies,
    ) = if let Some(expected_count) = expected_frame_count {
        let alignment = align_timeline(&temporal_observations, expected_count);
        (
            alignment.unique_valid_frames,
            alignment.deleted_frames + alignment.replaced_frames,
            alignment.inserted_frames,
            alignment.deleted_frames,
            alignment.duplicate_frames,
            alignment.out_of_order_frames,
            alignment.replaced_frames,
            alignment.unreadable_frames,
            alignment.anomalies,
        )
    } else {
        let unreadable = frames.len().saturating_sub(legacy_valid_frames);
        (
            legacy_valid_frames,
            unreadable,
            0,
            0,
            0,
            0,
            unreadable,
            unreadable,
            Vec::new(),
        )
    };

    let has_timeline_attack = missing_frames > 0
        || inserted_frames > 0
        || duplicate_frames > 0
        || out_of_order_frames > 0
        || replaced_frames > 0;
    let integrity = if expected_temporal.is_none() {
        // 旧视频没有帧序号、总帧数和片段 ID，因此即使每帧文本一致，也不能证明中间没有删帧或重复帧。
        // Legacy video lacks frame index, total count, and clip ID, so matching text on every frame cannot prove that no middle frame was dropped or duplicated.
        if replaced_frames > 0 || modified_frames > 0 || transformed_frames > 0 {
            IntegrityStatus::Modified
        } else {
            IntegrityStatus::Uncertain
        }
    } else if has_timeline_attack || modified_frames > 0 || transformed_frames > 0 {
        IntegrityStatus::Modified
    } else {
        IntegrityStatus::Intact
    };

    let text = expected_temporal
        .as_ref()
        .map(|(text, _, _)| text.clone())
        .or(expected_legacy_text)
        .unwrap_or_default();
    progress.emit(
        app,
        "completed-file",
        "当前视频解码检测完成",
        1.0,
        frames.len(),
        frames.len(),
    );
    Ok(VideoDecodeReport {
        text,
        integrity,
        corrected_codewords,
        frame_count: frames.len(),
        expected_frame_count,
        valid_frames,
        modified_frames,
        missing_frames,
        inserted_frames,
        deleted_frames,
        duplicate_frames,
        out_of_order_frames,
        replaced_frames,
        unreadable_frames,
        transformed_frames,
        timeline_anomalies,
        tamper_regions,
        sync_registration,
    })
}

#[derive(Debug, Default)]
struct TimelineAlignment {
    unique_valid_frames: usize,
    inserted_frames: usize,
    deleted_frames: usize,
    duplicate_frames: usize,
    out_of_order_frames: usize,
    replaced_frames: usize,
    unreadable_frames: usize,
    anomalies: Vec<TimelineAnomalyRange>,
}

fn align_timeline(observations: &[Option<usize>], expected_count: usize) -> TimelineAlignment {
    let mut alignment = TimelineAlignment::default();
    let mut present_indices: Vec<usize> = observations
        .iter()
        .flatten()
        .copied()
        .filter(|expected_index| *expected_index < expected_count)
        .collect();
    present_indices.sort_unstable();
    present_indices.dedup();
    alignment.unique_valid_frames = present_indices.len();
    alignment.unreadable_frames = observations.iter().filter(|value| value.is_none()).count();

    let aligned_physical = longest_increasing_observation_positions(observations);
    let mut canonical_positions = HashMap::<usize, usize>::with_capacity(present_indices.len());
    for &physical_index in &aligned_physical {
        if let Some(expected_index) = observations[physical_index] {
            canonical_positions.insert(expected_index, physical_index);
        }
    }
    for (physical_index, observation) in observations.iter().enumerate() {
        if let Some(expected_index) = *observation {
            canonical_positions
                .entry(expected_index)
                .or_insert(physical_index);
        }
    }

    let mut ordered_unique = Vec::with_capacity(alignment.unique_valid_frames);
    for (physical_index, observation) in observations.iter().enumerate() {
        let Some(expected_index) = *observation else {
            continue;
        };
        if canonical_positions.get(&expected_index) == Some(&physical_index) {
            ordered_unique.push((physical_index, expected_index));
        } else {
            alignment.duplicate_frames += 1;
            push_anomaly(
                &mut alignment.anomalies,
                single_frame_anomaly(
                    TimelineAnomalyKind::Duplicate,
                    Some(physical_index),
                    Some(expected_index),
                ),
            );
        }
    }

    if ordered_unique.len() > 1 {
        // 前缀最大值与后缀最小值可以在线性时间内标出所有参与逆序的帧，因此交换两帧会报告整个受影响区间，而不是只报告 LCS 排除的一个帧。
        // Prefix maxima and suffix minima mark every frame participating in an inversion in linear time, so swapping two frames reports the whole affected range rather than only one frame excluded by the LCS.
        let mut prefix_max = vec![0_usize; ordered_unique.len()];
        let mut suffix_min = vec![usize::MAX; ordered_unique.len()];
        for index in 0..ordered_unique.len() {
            prefix_max[index] = if index == 0 {
                ordered_unique[index].1
            } else {
                prefix_max[index - 1].max(ordered_unique[index].1)
            };
        }
        for index in (0..ordered_unique.len()).rev() {
            suffix_min[index] = if index + 1 == ordered_unique.len() {
                ordered_unique[index].1
            } else {
                suffix_min[index + 1].min(ordered_unique[index].1)
            };
        }
        for (index, (physical_index, expected_index)) in ordered_unique.iter().copied().enumerate()
        {
            let conflicts_with_previous = index > 0 && prefix_max[index - 1] > expected_index;
            let conflicts_with_following =
                index + 1 < ordered_unique.len() && suffix_min[index + 1] < expected_index;
            if conflicts_with_previous || conflicts_with_following {
                alignment.out_of_order_frames += 1;
                push_anomaly(
                    &mut alignment.anomalies,
                    single_frame_anomaly(
                        TimelineAnomalyKind::OutOfOrder,
                        Some(physical_index),
                        Some(expected_index),
                    ),
                );
            }
        }
    }

    let anchors: Vec<(usize, usize)> = aligned_physical
        .iter()
        .filter_map(|physical_index| {
            observations[*physical_index].map(|expected_index| (*physical_index, expected_index))
        })
        .collect();
    let mut previous_physical = None;
    let mut previous_expected = None;
    for anchor_index in 0..=anchors.len() {
        let next_anchor = anchors.get(anchor_index).copied();
        let physical_start = previous_physical.map_or(0, |value| value + 1);
        let physical_end = next_anchor.map_or(observations.len(), |(value, _)| value);
        let expected_start = previous_expected.map_or(0, |value| value + 1);
        let expected_end = next_anchor.map_or(expected_count, |(_, value)| value);
        let unknown_runs = collect_unknown_runs(observations, physical_start, physical_end);
        let missing_runs = collect_missing_runs(&present_indices, expected_start, expected_end);

        // 同一对齐锚点区间内，“一个未匹配物理帧 + 一个缺失原始帧”按最小编辑解释为替换；区间游标可直接处理超长连续攻击而无需逐帧分配内存。
        // Within one alignment-anchor gap, one unmatched physical frame plus one absent original frame is the minimum-edit interpretation of a replacement; range cursors handle long attacks without per-frame allocation.
        align_gap_runs(unknown_runs, missing_runs, &mut alignment);

        if let Some((physical_index, expected_index)) = next_anchor {
            previous_physical = Some(physical_index);
            previous_expected = Some(expected_index);
        }
    }

    // 唯一帧序号的 LCS 等价于严格递增子序列；稀疏集合与区间对齐使内存由实际观测帧数决定，而不是由可伪造的声明总帧数决定。
    // With unique frame indices, LCS is equivalent to a strictly increasing subsequence; sparse sets and range alignment make memory depend on observed frames rather than a potentially forged declared total.
    alignment.anomalies.sort_by_key(|range| {
        (
            range.physical_start_frame.unwrap_or(usize::MAX),
            range.expected_start_frame.unwrap_or(usize::MAX),
        )
    });
    alignment
}

#[derive(Debug, Clone, Copy)]
struct IndexRun {
    start: usize,
    len: usize,
}

fn collect_unknown_runs(observations: &[Option<usize>], start: usize, end: usize) -> Vec<IndexRun> {
    let mut runs = Vec::new();
    let mut cursor = start;
    while cursor < end {
        if observations[cursor].is_some() {
            cursor += 1;
            continue;
        }
        let run_start = cursor;
        while cursor < end && observations[cursor].is_none() {
            cursor += 1;
        }
        runs.push(IndexRun {
            start: run_start,
            len: cursor - run_start,
        });
    }
    runs
}

fn collect_missing_runs(present_indices: &[usize], start: usize, end: usize) -> Vec<IndexRun> {
    if start >= end {
        return Vec::new();
    }
    let first = lower_bound(present_indices, start);
    let last = lower_bound(present_indices, end);
    let mut runs = Vec::new();
    let mut cursor = start;
    for &present in &present_indices[first..last] {
        if cursor < present {
            runs.push(IndexRun {
                start: cursor,
                len: present - cursor,
            });
        }
        cursor = present.saturating_add(1);
    }
    if cursor < end {
        runs.push(IndexRun {
            start: cursor,
            len: end - cursor,
        });
    }
    runs
}

fn align_gap_runs(
    unknown_runs: Vec<IndexRun>,
    missing_runs: Vec<IndexRun>,
    alignment: &mut TimelineAlignment,
) {
    let mut unknown_index = 0_usize;
    let mut missing_index = 0_usize;
    let mut unknown_offset = 0_usize;
    let mut missing_offset = 0_usize;

    while unknown_index < unknown_runs.len() && missing_index < missing_runs.len() {
        let unknown = unknown_runs[unknown_index];
        let missing = missing_runs[missing_index];
        let unknown_remaining = unknown.len - unknown_offset;
        let missing_remaining = missing.len - missing_offset;
        let paired = unknown_remaining.min(missing_remaining);
        alignment.replaced_frames += paired;
        push_anomaly(
            &mut alignment.anomalies,
            range_anomaly(
                TimelineAnomalyKind::Replacement,
                Some(unknown.start + unknown_offset),
                Some(missing.start + missing_offset),
                paired,
            ),
        );
        unknown_offset += paired;
        missing_offset += paired;
        if unknown_offset == unknown.len {
            unknown_index += 1;
            unknown_offset = 0;
        }
        if missing_offset == missing.len {
            missing_index += 1;
            missing_offset = 0;
        }
    }

    if unknown_index < unknown_runs.len() && unknown_offset > 0 {
        let run = unknown_runs[unknown_index];
        let remaining = run.len - unknown_offset;
        alignment.inserted_frames += remaining;
        push_anomaly(
            &mut alignment.anomalies,
            range_anomaly(
                TimelineAnomalyKind::Insertion,
                Some(run.start + unknown_offset),
                None,
                remaining,
            ),
        );
        unknown_index += 1;
    }
    for run in &unknown_runs[unknown_index..] {
        alignment.inserted_frames += run.len;
        push_anomaly(
            &mut alignment.anomalies,
            range_anomaly(
                TimelineAnomalyKind::Insertion,
                Some(run.start),
                None,
                run.len,
            ),
        );
    }

    if missing_index < missing_runs.len() && missing_offset > 0 {
        let run = missing_runs[missing_index];
        let remaining = run.len - missing_offset;
        alignment.deleted_frames += remaining;
        push_anomaly(
            &mut alignment.anomalies,
            range_anomaly(
                TimelineAnomalyKind::Deletion,
                None,
                Some(run.start + missing_offset),
                remaining,
            ),
        );
        missing_index += 1;
    }
    for run in &missing_runs[missing_index..] {
        alignment.deleted_frames += run.len;
        push_anomaly(
            &mut alignment.anomalies,
            range_anomaly(
                TimelineAnomalyKind::Deletion,
                None,
                Some(run.start),
                run.len,
            ),
        );
    }
}

fn longest_increasing_observation_positions(observations: &[Option<usize>]) -> Vec<usize> {
    let sequence: Vec<(usize, usize)> = observations
        .iter()
        .enumerate()
        .filter_map(|(physical_index, expected_index)| {
            (*expected_index).map(|value| (physical_index, value))
        })
        .collect();
    if sequence.is_empty() {
        return Vec::new();
    }

    let mut tail_values = Vec::new();
    let mut tail_sequence_positions = Vec::new();
    let mut predecessors = vec![None; sequence.len()];
    for (sequence_position, (_, expected_index)) in sequence.iter().enumerate() {
        let insertion = lower_bound(&tail_values, *expected_index);
        if insertion > 0 {
            predecessors[sequence_position] = Some(tail_sequence_positions[insertion - 1]);
        }
        if insertion == tail_values.len() {
            tail_values.push(*expected_index);
            tail_sequence_positions.push(sequence_position);
        } else if tail_values[insertion] != *expected_index {
            // 相同帧号出现多次时保留最早的对齐位置，是为了把后插入的副本准确标为重复帧，而不是反过来指认原始帧。
            // When the same frame index appears repeatedly, keeping the earliest aligned position ensures a later inserted copy is reported as the duplicate rather than blaming the original frame.
            tail_values[insertion] = *expected_index;
            tail_sequence_positions[insertion] = sequence_position;
        }
    }

    let mut aligned = Vec::with_capacity(tail_values.len());
    let mut cursor = tail_sequence_positions.last().copied();
    while let Some(sequence_position) = cursor {
        aligned.push(sequence[sequence_position].0);
        cursor = predecessors[sequence_position];
    }
    aligned.reverse();
    aligned
}

fn lower_bound(values: &[usize], target: usize) -> usize {
    let mut left = 0_usize;
    let mut right = values.len();
    while left < right {
        let middle = left + (right - left) / 2;
        if values[middle] < target {
            left = middle + 1;
        } else {
            right = middle;
        }
    }
    left
}

fn single_frame_anomaly(
    kind: TimelineAnomalyKind,
    physical_index: Option<usize>,
    expected_index: Option<usize>,
) -> TimelineAnomalyRange {
    range_anomaly(kind, physical_index, expected_index, 1)
}

fn range_anomaly(
    kind: TimelineAnomalyKind,
    physical_start: Option<usize>,
    expected_start: Option<usize>,
    count: usize,
) -> TimelineAnomalyRange {
    debug_assert!(count > 0);
    // 对外使用从 1 开始的闭区间帧号，是为了与播放器和剪辑软件的用户界面保持一致。
    // Public frame numbers use one-based inclusive ranges to match the convention used by video players and editors.
    TimelineAnomalyRange {
        kind,
        physical_start_frame: physical_start.map(|value| value.saturating_add(1)),
        physical_end_frame: physical_start.map(|value| value.saturating_add(count)),
        expected_start_frame: expected_start.map(|value| value.saturating_add(1)),
        expected_end_frame: expected_start.map(|value| value.saturating_add(count)),
        count,
    }
}

fn push_anomaly(ranges: &mut Vec<TimelineAnomalyRange>, next: TimelineAnomalyRange) {
    if let Some(last) = ranges.last_mut() {
        let physical_contiguous =
            ranges_are_contiguous(last.physical_end_frame, next.physical_start_frame);
        let expected_contiguous =
            ranges_are_contiguous(last.expected_end_frame, next.expected_start_frame);
        let can_merge = match next.kind {
            TimelineAnomalyKind::Insertion => {
                physical_contiguous
                    && last.expected_start_frame.is_none()
                    && next.expected_start_frame.is_none()
            }
            TimelineAnomalyKind::Deletion => {
                expected_contiguous
                    && last.physical_start_frame.is_none()
                    && next.physical_start_frame.is_none()
            }
            TimelineAnomalyKind::Replacement => physical_contiguous && expected_contiguous,
            TimelineAnomalyKind::Duplicate | TimelineAnomalyKind::OutOfOrder => {
                // 连续的重复/错序物理帧合并为一个区间，是为了避免恶意长片段产生逐帧 JSON；原始帧范围使用最小值到最大值表达覆盖面。
                // Consecutive duplicate/out-of-order physical frames are merged to prevent per-frame JSON growth on long attacks; the expected range stores their minimum-to-maximum coverage.
                physical_contiguous
            }
        };
        if last.kind == next.kind && can_merge {
            last.physical_start_frame =
                optional_min(last.physical_start_frame, next.physical_start_frame);
            last.physical_end_frame =
                optional_max(last.physical_end_frame, next.physical_end_frame);
            last.expected_start_frame =
                optional_min(last.expected_start_frame, next.expected_start_frame);
            last.expected_end_frame =
                optional_max(last.expected_end_frame, next.expected_end_frame);
            last.count += next.count;
            return;
        }
    }
    ranges.push(next);
}

fn optional_min(left: Option<usize>, right: Option<usize>) -> Option<usize> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn optional_max(left: Option<usize>, right: Option<usize>) -> Option<usize> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn ranges_are_contiguous(left: Option<usize>, right: Option<usize>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.saturating_add(1) == right,
        (None, None) => true,
        _ => false,
    }
}

fn normalize_parallelism(requested: usize) -> usize {
    let available = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let requested = requested.clamp(1, available.max(1));
    // 限制并行度不超过可用 CPU 数，是为了避免高清视频抽帧后同时打开过多 PNG 导致磁盘与内存抖动。
    // Parallelism is capped at available CPUs to avoid disk and memory thrashing when many extracted PNG frames are processed.
    requested
}

#[cfg(unix)]
fn ensure_program_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .map_err(|error| format!("无法读取 FFmpeg 文件权限 {}：{error}", path.display()))?;
    let mode = metadata.permissions().mode();
    if mode & 0o755 == 0o755 {
        return Ok(());
    }

    let mut permissions = metadata.permissions();
    permissions.set_mode(mode | 0o755);
    // 视频任务真正启动 FFmpeg 前再修复一次执行权限，避免 manifest 阶段与运行阶段资源目录不同导致 macOS 报 Permission denied。
    // The executable bit is fixed right before video tasks start so macOS does not fail with Permission denied when manifest and runtime resource paths differ.
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("无法设置 FFmpeg 可执行权限 {}：{error}", path.display()))
}

#[cfg(not(unix))]
fn ensure_program_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    // FFmpeg/FFprobe 是内部后台工具，隐藏控制台窗口可避免桌面端用户看到闪现的命令行。
    // FFmpeg/FFprobe are internal background tools, so hiding the console prevents command windows from flashing in the desktop app.
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(target_os = "windows"))]
fn hidden_command(program: &Path) -> Command {
    // 非 Windows 平台没有 creation_flags，直接返回 Command 可以避免 macOS/Linux 构建产生无意义的 mut warning。
    // Non-Windows platforms do not use creation_flags, so returning Command directly avoids a meaningless mut warning on macOS/Linux builds.
    Command::new(program)
}

fn run_ffmpeg_with_progress<I>(
    app: &AppHandle,
    progress: &VideoProgressContext,
    cancellation: &CancellationToken,
    phase: &str,
    heartbeat_message: &str,
    start_fraction: f64,
    end_fraction: f64,
    current: usize,
    total: usize,
    observed_output_dir: Option<PathBuf>,
    media_duration_seconds: Option<f64>,
    program: &Path,
    args: I,
) -> Result<(), String>
where
    I: IntoIterator<Item = OsString>,
{
    cancellation.check()?;
    let args: Vec<OsString> = args.into_iter().collect();
    ensure_program_executable(program)?;
    let mut child = hidden_command(program)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 {}：{error}", program.display()))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (progress_tx, progress_rx) = mpsc::channel::<String>();
    let stdout_reader = thread::spawn(move || {
        if let Some(stdout) = stdout {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = progress_tx.send(line);
            }
        }
    });
    let stderr_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_end(&mut buffer);
        }
        String::from_utf8_lossy(&buffer).trim().to_owned()
    });

    let started_at = Instant::now();
    let mut next_heartbeat = Instant::now();
    let duration_micros = media_duration_seconds
        .filter(|value| *value > 0.0)
        .map(|value| value * 1_000_000.0);
    let mut latest_ratio = 0.0_f64;
    let mut latest_frame = None;
    let mut observed_count = current;
    let mut next_directory_probe = Instant::now() + Duration::from_secs(5);

    loop {
        if cancellation.is_cancelled() {
            // 取消时主动杀掉 FFmpeg 子进程，否则 Rust 任务返回后外部进程仍可能继续占用磁盘并写临时帧。
            // On cancellation the FFmpeg child is killed explicitly so it cannot keep using disk or writing temporary frames after Rust returns.
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("任务已取消".into());
        }

        // FFmpeg 已提供帧计数；只合并最新状态并按心跳发送，避免每一行进度都遍历不断增长的帧目录。
        // FFmpeg already reports frame counts; coalescing state into heartbeat events avoids rescanning a growing directory for every progress line.
        while let Ok(line) = progress_rx.try_recv() {
            if let Some(frame) = parse_ffmpeg_frame_count(&line) {
                latest_frame = Some(frame);
                observed_count = frame;
            }
            if let Some(ratio) = parse_ffmpeg_progress_ratio(&line, duration_micros) {
                latest_ratio = latest_ratio.max(ratio);
            }
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("等待 {} 执行结果失败：{error}", program.display()))?
        {
            let _ = stdout_reader.join();
            let stderr = stderr_reader.join().unwrap_or_else(|_| String::new());
            if status.success() {
                progress.emit(
                    app,
                    phase,
                    heartbeat_message,
                    end_fraction,
                    observed_count,
                    total,
                );
                return Ok(());
            }
            return Err(format!(
                "FFmpeg 命令执行失败（退出码 {:?}）：{}",
                status.code(),
                stderr
            ));
        }

        if Instant::now() >= next_heartbeat {
            let elapsed = started_at.elapsed().as_secs();
            // 抽帧和封装有时不能稳定输出逐行进度，因此仍保留心跳事件，避免界面看起来像完全卡死。
            // Extraction and muxing do not always emit stable line-by-line progress, so heartbeat updates remain as a fallback to avoid a frozen-looking UI.
            let file_fraction = start_fraction + (end_fraction - start_fraction) * latest_ratio;
            if latest_frame.is_none() && Instant::now() >= next_directory_probe {
                observed_count = observed_output_dir
                    .as_deref()
                    .map(count_png_files)
                    .unwrap_or(current);
                next_directory_probe = Instant::now() + Duration::from_secs(5);
            }
            let generated_suffix = observed_output_dir
                .as_ref()
                .map(|_| format!("，已生成 {observed_count} 帧"))
                .unwrap_or_default();
            progress.emit(
                app,
                phase,
                format!("{heartbeat_message}，已运行 {elapsed} 秒{generated_suffix}"),
                file_fraction,
                observed_count,
                total,
            );
            next_heartbeat = Instant::now() + Duration::from_millis(500);
        }

        thread::sleep(Duration::from_millis(200));
    }
}

fn count_png_files(directory: &Path) -> usize {
    fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(std::result::Result::ok)
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|extension| extension.eq_ignore_ascii_case("png"))
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn parse_ffmpeg_frame_count(line: &str) -> Option<usize> {
    line.strip_prefix("frame=")?.trim().parse().ok()
}

fn parse_ffmpeg_progress_ratio(line: &str, duration_micros: Option<f64>) -> Option<f64> {
    let duration_micros = duration_micros?;
    let value = if let Some(raw) = line.strip_prefix("out_time_ms=") {
        raw.trim().parse::<f64>().ok()
    } else if let Some(raw) = line.strip_prefix("out_time_us=") {
        raw.trim().parse::<f64>().ok()
    } else {
        None
    }?;
    Some((value / duration_micros).clamp(0.0, 1.0))
}

fn probe_media_duration_seconds(ffprobe: &Path, input: &Path) -> Result<Option<f64>, String> {
    ensure_program_executable(ffprobe)?;
    let output = hidden_command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(ffmpeg_input_arg(input))
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Ok(None);
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(raw.parse::<f64>().ok())
}

fn probe_frame_rate(ffprobe: &Path, input: &Path) -> Result<String, String> {
    ensure_program_executable(ffprobe)?;
    let output = hidden_command(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(ffmpeg_input_arg(input))
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let rate = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if rate.is_empty() || rate == "0/0" {
        Ok("30".into())
    } else {
        Ok(rate)
    }
}

fn probe_png_size(path: &Path) -> Result<(u32, u32), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() < 24 || bytes.get(0..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err("无法读取视频帧尺寸".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    Ok((width, height))
}

fn verify_watermarked_video_frames(
    app: &AppHandle,
    ffmpeg: &Path,
    output: &Path,
    key: &WatermarkKey,
    frame_count: usize,
    progress: &VideoProgressContext,
    cancellation: &CancellationToken,
) -> Result<(), String> {
    let work_dir = storage::create_work_dir("video_verify").map_err(|error| error.to_string())?;
    let mut indices = vec![0, frame_count / 2, frame_count.saturating_sub(1)];
    indices.sort_unstable();
    indices.dedup();
    let select = indices
        .iter()
        .map(|index| format!("eq(n\\,{index})"))
        .collect::<Vec<_>>()
        .join("+");
    // 一次顺序解码取得去重后的首中尾样本，避免三次从头读取视频，并沿用可取消的子进程管理。
    // One sequential decode selects deduplicated first/middle/last samples, avoiding three full-prefix reads and retaining cancellable child-process management.
    run_ffmpeg_with_progress(
        app,
        progress,
        cancellation,
        "verifying-video",
        "正在回读输出视频",
        0.98,
        0.99,
        0,
        indices.len(),
        None,
        None,
        ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("error"),
            OsString::from("-nostats"),
            OsString::from("-progress"),
            OsString::from("pipe:1"),
            OsString::from("-y"),
            OsString::from("-i"),
            ffmpeg_input_arg(output),
            OsString::from("-vf"),
            OsString::from(format!("select={select}")),
            OsString::from("-fps_mode"),
            OsString::from("vfr"),
            OsString::from("-frames:v"),
            OsString::from(indices.len().to_string()),
            work_dir.path().join("check_%03d.png").into_os_string(),
        ],
    )?;
    for slot in 1..=indices.len() {
        cancellation.check()?;
        let result = extract_image_file_with_recovery(
            work_dir.path().join(format!("check_{slot:03}.png")),
            &KeySource::KeyFile(key.to_key_file()),
            ExtractOptions {
                allow_registration: false,
            },
            RecoveryOptions {
                cancellation: Some(cancellation.callback()),
                ..Default::default()
            },
            |_| Ok(()),
        );
        match result {
            Ok(_) => {}
            Err(CoreError::Cancelled) => return Err("任务已取消".into()),
            Err(_) => {
                let _ = fs::remove_file(output);
                return Err("封装后无法回读，未生成密钥".into());
            }
        }
    }
    Ok(())
}

fn sorted_png_files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(directory)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png"))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod timeline_tests {
    use super::*;

    #[test]
    fn timeline_alignment_distinguishes_insertions() {
        let result = align_timeline(&[Some(0), Some(1), None, None, Some(2), Some(3)], 4);
        assert_eq!(result.inserted_frames, 2);
        assert_eq!(result.deleted_frames, 0);
        assert_eq!(result.replaced_frames, 0);
        assert_eq!(result.unique_valid_frames, 4);
        assert_eq!(result.anomalies.len(), 1);
        assert_eq!(result.anomalies[0].kind, TimelineAnomalyKind::Insertion);
        assert_eq!(result.anomalies[0].physical_start_frame, Some(3));
        assert_eq!(result.anomalies[0].physical_end_frame, Some(4));
    }

    #[test]
    fn timeline_alignment_distinguishes_replacement_and_deletion() {
        let replaced = align_timeline(&[Some(0), None, Some(2), Some(3)], 4);
        assert_eq!(replaced.replaced_frames, 1);
        assert_eq!(replaced.deleted_frames, 0);
        assert_eq!(replaced.inserted_frames, 0);

        let deleted = align_timeline(&[Some(0), Some(2), Some(3)], 4);
        assert_eq!(deleted.deleted_frames, 1);
        assert_eq!(deleted.replaced_frames, 0);
    }

    #[test]
    fn timeline_alignment_marks_duplicates_and_reordering() {
        let duplicate = align_timeline(&[Some(0), Some(1), Some(1), Some(2), Some(3)], 4);
        assert_eq!(duplicate.duplicate_frames, 1);
        assert_eq!(duplicate.out_of_order_frames, 0);
        assert_eq!(duplicate.anomalies[0].physical_start_frame, Some(3));

        let reordered = align_timeline(&[Some(0), Some(2), Some(1), Some(3)], 4);
        assert_eq!(reordered.out_of_order_frames, 2);
        assert_eq!(reordered.duplicate_frames, 0);
        let range = reordered
            .anomalies
            .iter()
            .find(|range| range.kind == TimelineAnomalyKind::OutOfOrder)
            .expect("错序区间应存在");
        assert_eq!(range.physical_start_frame, Some(2));
        assert_eq!(range.physical_end_frame, Some(3));
        assert_eq!(range.expected_start_frame, Some(2));
        assert_eq!(range.expected_end_frame, Some(3));
    }

    #[test]
    fn timeline_alignment_keeps_insertions_and_missing_originals_separate() {
        let result = align_timeline(&[Some(0), None, Some(1), Some(3), None, Some(4)], 5);
        assert_eq!(result.inserted_frames, 2);
        assert_eq!(result.deleted_frames, 1);
        assert_eq!(result.replaced_frames, 0);
        assert_eq!(result.unique_valid_frames, 4);
    }

    #[test]
    fn timeline_alignment_keeps_declared_frame_count_sparse() {
        let result = align_timeline(&[Some(0), Some(999_999_999)], 1_000_000_000);
        assert_eq!(result.unique_valid_frames, 2);
        assert_eq!(result.deleted_frames, 999_999_998);
        assert_eq!(result.anomalies.len(), 1);
        assert_eq!(result.anomalies[0].expected_start_frame, Some(2));
        assert_eq!(result.anomalies[0].expected_end_frame, Some(999_999_999));
    }
}

#[cfg(test)]
mod encode_guard_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn ffmpeg_input_arg_does_not_pass_dash_filename_as_flag() {
        let arg = ffmpeg_input_arg(Path::new("-evil.mp4"));
        let text = arg.to_string_lossy();
        assert!(
            !text.starts_with("-e"),
            "dash-prefixed input must not become an ffmpeg flag: {text}"
        );
        assert!(text.contains("-evil.mp4"));
    }

    #[test]
    fn effective_video_strength_keeps_user_eight() {
        assert!((effective_video_strength(8.0) - 8.0).abs() < f32::EPSILON);
        assert!((effective_video_strength(1.0) - 3.0).abs() < f32::EPSILON);
        assert!((effective_video_strength(32.0) - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn consecutive_frame_streak_does_not_abort_when_rate_is_under_ten_percent() {
        assert!(!video_frame_failure_aborts_clip(8, 246));
        assert!(!video_frame_failure_aborts_clip(21, 246));
        assert!(video_frame_failure_aborts_clip(25, 246));
        assert!(video_frame_failure_aborts_clip(2, 10));
    }

    #[test]
    fn video_pixel_budget_rejects_oversize_clips() {
        assert!(!exceeds_max_video_pixel_frames(1920, 1080, 100));
        assert!(exceeds_max_video_pixel_frames(1920, 1080, 20_000));
    }
}
