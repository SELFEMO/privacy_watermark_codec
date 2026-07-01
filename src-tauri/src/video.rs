use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use tracing::{info, warn};
use walkdir::WalkDir;
use watermark_core::{
    embed_image_file, extract_image_file_with_options, EmbedOptions, ExtractOptions, IntegrityStatus,
    KeySource, SyncRegistration, TamperRegion, WatermarkKey,
};

use crate::{
    cancellation::CancellationToken,
    ffmpeg,
    models::{TaskProgressEvent, TaskProgressKind},
    progress::{clamp_percent, emit_task_progress},
    storage,
};
use tauri::AppHandle;

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
        let total_files = self.file_total.max(1) as f64;
        let percent = ((self.file_index as f64 + file_fraction.clamp(0.0, 1.0)) / total_files) * 100.0;
        emit_task_progress(
            app,
            TaskProgressEvent {
                task_id: self.task_id.clone(),
                task: self.task,
                phase: phase.to_owned(),
                message: message.into(),
                current,
                total,
                percent: clamp_percent(percent),
                current_path: Some(self.file_path.clone()),
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
    pub valid_frames: usize,
    pub modified_frames: usize,
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
    let input_duration_seconds = probe_media_duration_seconds(&tools.ffprobe, input).ok().flatten();
    progress.emit(app, "extracting-video-frames", "正在使用 FFmpeg 抽取视频帧", 0.04, 0, 0);
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
            input.as_os_str().to_owned(),
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
    progress.emit(
        app,
        "processing-video-frames",
        "视频帧已抽取，开始逐帧嵌入水印",
        0.30,
        0,
        frames.len(),
    );

    let parallelism = normalize_parallelism(options.frame_parallelism);
    for (chunk_index, chunk) in frames.chunks(parallelism).enumerate() {
        cancellation.check()?;
        thread::scope(|scope| {
            let mut handles = Vec::with_capacity(chunk.len());
            for frame in chunk {
                let frame = frame.clone();
                let output_frame = marked_frames.join(frame.file_name().unwrap());
                let key = key.clone();
                let text = text.to_owned();
                handles.push(scope.spawn(move || {
                    let options = EmbedOptions {
                        text,
                        key,
                        // 视频会经历颜色空间转换与 H.264 再编码，因此使用更高的最低强度为后续压缩损耗预留安全余量。
                        // Video goes through color-space conversion and H.264 re-encoding, so a higher minimum strength leaves margin for later compression loss.
                        strength: strength.max(16.0),
                        media_kind: "video_frame".into(),
                    };
                    embed_image_file(&frame, &output_frame, &options).map_err(|error| error.to_string())
                }));
            }

            for handle in handles {
                handle
                    .join()
                    .map_err(|_| "视频帧编码线程异常终止".to_owned())??;
            }
            Ok::<(), String>(())
        })?;
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
        info!(processed, total = frames.len(), parallelism, "视频帧水印处理进度");
    }

    cancellation.check()?;
    let frame_rate = probe_frame_rate(&tools.ffprobe, input)?;
    info!(frame_rate = %frame_rate, frames = frames.len(), "开始重新编码视频并复制音轨");
    // 使用 H.264 lossless + yuv444p 是为了尽量保留亮度频域系数，否则本软件刚生成的视频也可能在再次解码时丢失水印。
    // H.264 lossless with yuv444p preserves luminance-frequency coefficients as much as possible; otherwise even videos generated by this app may lose the watermark during decoding.
    progress.emit(app, "muxing-video", "正在重新封装视频并复制音轨", 0.86, frames.len(), frames.len());
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
            input.as_os_str().to_owned(),
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

    progress.emit(app, "completed-file", "当前视频编码完成", 1.0, frames.len(), frames.len());
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
    cancellation: &CancellationToken,
    progress: VideoProgressContext,
) -> Result<VideoDecodeReport, String> {
    let tools = ffmpeg::bundled_tools(app)?;
    let work_dir = storage::create_work_dir("video_decode").map_err(|error| error.to_string())?;
    let frames_dir = work_dir.path().join("frames");
    fs::create_dir_all(&frames_dir).map_err(|error| error.to_string())?;

    let input_duration_seconds = probe_media_duration_seconds(&tools.ffprobe, input).ok().flatten();
    progress.emit(app, "extracting-video-frames", "正在使用 FFmpeg 抽取待检测视频帧", 0.04, 0, 0);
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
            input.as_os_str().to_owned(),
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
                handles.push(scope.spawn(move || {
                    let frame_index = start_index + offset;
                    let options = ExtractOptions {
                        // 视频编码不会产生旋转或缩放，关闭逐帧配准可以避免首个并行批次因多候选重采样而长时间无进度。
                        // Encoding does not rotate or scale video frames, so disabling per-frame registration prevents the first parallel batch from spending a long time on resampling candidates.
                        allow_registration: false,
                    };
                    (frame_index, extract_image_file_with_options(&frame, &key_source, options))
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
        cancellation.check()?;
        reports.append(&mut chunk_reports);

        let processed = ((chunk_index + 1) * parallelism).min(frames.len());
        let frame_fraction = processed as f64 / frames.len() as f64;
        progress.emit(
            app,
            "checking-video-frames",
            format!("正在逐帧解码检测：{processed}/{}", frames.len()),
            0.30 + frame_fraction * 0.64,
            processed,
            frames.len(),
        );
        info!(processed, total = frames.len(), parallelism, "视频逐帧检测进度");
    }
    reports.sort_by_key(|(index, _)| *index);
    progress.emit(app, "summarizing-video", "正在汇总视频逐帧检测结果", 0.96, frames.len(), frames.len());

    let mut expected_text: Option<String> = None;
    let mut valid_frames = 0_usize;
    let mut modified_frames = 0_usize;
    let mut corrected_codewords = 0_usize;
    let mut tamper_regions = Vec::new();
    let mut sync_registration = None;

    for (index, result) in reports {
        cancellation.check()?;
        match result {
            Ok(report) => {
                if let Some(text) = &expected_text {
                    if text != &report.text {
                        warn!(frame = index + 1, "视频帧提取出的水印文本不一致");
                        continue;
                    }
                } else {
                    expected_text = Some(report.text.clone());
                    sync_registration = Some(report.sync_registration);
                }
                valid_frames += 1;
                corrected_codewords += report.corrected_codewords;
                if report.integrity != IntegrityStatus::Intact {
                    modified_frames += 1;
                }
                tamper_regions.extend(report.tamper_regions);
            }
            Err(error) => {
                warn!(frame = index + 1, %error, "视频帧水印提取失败");
            }
        }
    }

    if valid_frames == 0 {
        return Err(format!(
            "视频逐帧校验失败：共 {} 帧，没有任何帧成功提取一致水印",
            frames.len()
        ));
    }

    let missing_frames = frames.len().saturating_sub(valid_frames);
    let integrity = if missing_frames == 0 {
        IntegrityStatus::Intact
    } else if valid_frames * 5 >= frames.len() * 4 {
        IntegrityStatus::Uncertain
    } else {
        IntegrityStatus::Modified
    };
    // 视频压缩会改变感知指纹，逐帧指纹不再作为“解码成功/失败”的硬条件；用水印文本一致性和缺失帧比例判断证据强度。
    // Video compression changes perceptual fingerprints, so frame fingerprints are not a hard decode condition; text consistency and missing-frame ratio determine evidence strength.
    if missing_frames > 0 {
        warn!(
            valid_frames,
            total_frames = frames.len(),
            missing_frames,
            ?integrity,
            "视频水印只在部分帧中成功提取"
        );
    }

    progress.emit(app, "completed-file", "当前视频解码检测完成", 1.0, frames.len(), frames.len());
    Ok(VideoDecodeReport {
        text: expected_text.unwrap_or_default(),
        integrity,
        corrected_codewords,
        frame_count: frames.len(),
        valid_frames,
        modified_frames,
        tamper_regions,
        sync_registration,
    })
}

fn normalize_parallelism(requested: usize) -> usize {
    let available = thread::available_parallelism().map(usize::from).unwrap_or(1);
    let requested = requested.clamp(1, available.max(1));
    // 限制并行度不超过可用 CPU 数，是为了避免高清视频抽帧后同时打开过多 PNG 导致磁盘与内存抖动。
    // Parallelism is capped at available CPUs to avoid disk and memory thrashing when many extracted PNG frames are processed.
    requested
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // FFmpeg/FFprobe 是内部后台工具，隐藏控制台窗口可避免桌面端用户看到闪现的命令行。
        // FFmpeg/FFprobe are internal background tools, so hiding the console prevents command windows from flashing in the desktop app.
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
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
    media_duration_seconds: Option<f64>,
    program: &Path,
    args: I,
) -> Result<(), String>
where
    I: IntoIterator<Item = OsString>,
{
    cancellation.check()?;
    let args: Vec<OsString> = args.into_iter().collect();
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

        while let Ok(line) = progress_rx.try_recv() {
            if let Some(progress_ratio) = parse_ffmpeg_progress_ratio(&line, duration_micros) {
                latest_ratio = progress_ratio;
                let elapsed = started_at.elapsed().as_secs();
                let file_fraction = start_fraction + (end_fraction - start_fraction) * progress_ratio;
                progress.emit(
                    app,
                    phase,
                    format!("{heartbeat_message}，阶段进度 {:.0}% ，已运行 {elapsed} 秒", progress_ratio * 100.0),
                    file_fraction,
                    current,
                    total,
                );
            }
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("等待 {} 执行结果失败：{error}", program.display()))?
        {
            let _ = stdout_reader.join();
            let stderr = stderr_reader.join().unwrap_or_else(|_| String::new());
            if status.success() {
                if latest_ratio < 1.0 {
                    progress.emit(app, phase, heartbeat_message, end_fraction, current, total);
                }
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
            progress.emit(
                app,
                phase,
                format!("{heartbeat_message}，已运行 {elapsed} 秒"),
                file_fraction,
                current,
                total,
            );
            next_heartbeat = Instant::now() + Duration::from_secs(1);
        }

        thread::sleep(Duration::from_millis(200));
    }
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
    let output = hidden_command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
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
        .arg(input)
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
