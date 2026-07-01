use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use tracing::{info, warn};
use walkdir::WalkDir;
use watermark_core::{
    embed_image_file, extract_image_file, EmbedOptions, IntegrityStatus, KeySource, WatermarkKey,
};

use crate::{ffmpeg, storage};
use tauri::AppHandle;

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
}

pub fn encode_video(
    app: &AppHandle,
    input: &Path,
    output: &Path,
    text: &str,
    key: &WatermarkKey,
    strength: f32,
) -> Result<VideoEncodeReport, String> {
    let tools = ffmpeg::bundled_tools(app)?;
    let work_dir = storage::create_work_dir("video_encode").map_err(|error| error.to_string())?;
    let source_frames = work_dir.path().join("source");
    let marked_frames = work_dir.path().join("marked");
    fs::create_dir_all(&source_frames).map_err(|error| error.to_string())?;
    fs::create_dir_all(&marked_frames).map_err(|error| error.to_string())?;

    info!(input = %input.display(), "开始使用 FFmpeg 解码视频帧");
    run_command(
        &tools.ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("warning"),
            OsString::from("-y"),
            OsString::from("-i"),
            input.as_os_str().to_owned(),
            OsString::from("-vsync"),
            OsString::from("0"),
            source_frames.join("frame_%09d.png").into_os_string(),
        ],
    )?;

    let frames = sorted_png_files(&source_frames);
    if frames.is_empty() {
        return Err("FFmpeg 未能从视频中解码出任何画面帧".into());
    }

    for (index, frame) in frames.iter().enumerate() {
        let output_frame = marked_frames.join(frame.file_name().unwrap());
        let options = EmbedOptions {
            text: text.to_owned(),
            key: key.clone(),
            // 视频再次编码会削弱频域系数，因此最低使用 11 的强度保证常见 H.264 压缩后的可提取性。
            // Video re-encoding weakens frequency coefficients, so a minimum strength of 11 improves extraction after common H.264 compression.
            strength: strength.max(11.0),
            media_kind: "video_frame".into(),
        };
        embed_image_file(frame, &output_frame, &options).map_err(|error| error.to_string())?;
        if (index + 1) % 25 == 0 || index + 1 == frames.len() {
            info!(processed = index + 1, total = frames.len(), "视频帧水印处理进度");
        }
    }

    let frame_rate = probe_frame_rate(&tools.ffprobe, input)?;
    info!(frame_rate = %frame_rate, frames = frames.len(), "开始重新编码视频并复制音轨");
    run_command(
        &tools.ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("warning"),
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
            OsString::from("slow"),
            OsString::from("-crf"),
            OsString::from("12"),
            OsString::from("-pix_fmt"),
            OsString::from("yuv420p"),
            OsString::from("-c:a"),
            OsString::from("copy"),
            OsString::from("-shortest"),
            output.as_os_str().to_owned(),
        ],
    )?;

    info!(output = %output.display(), frames = frames.len(), "视频水印编码完成");
    Ok(VideoEncodeReport {
        frame_count: frames.len(),
    })
}

pub fn decode_video(
    app: &AppHandle,
    input: &Path,
    key_source: &KeySource,
) -> Result<VideoDecodeReport, String> {
    let tools = ffmpeg::bundled_tools(app)?;
    let work_dir = storage::create_work_dir("video_decode").map_err(|error| error.to_string())?;
    let frames_dir = work_dir.path().join("frames");
    fs::create_dir_all(&frames_dir).map_err(|error| error.to_string())?;

    run_command(
        &tools.ffmpeg,
        [
            OsString::from("-hide_banner"),
            OsString::from("-loglevel"),
            OsString::from("warning"),
            OsString::from("-y"),
            OsString::from("-i"),
            input.as_os_str().to_owned(),
            OsString::from("-vsync"),
            OsString::from("0"),
            frames_dir.join("frame_%09d.png").into_os_string(),
        ],
    )?;

    let frames = sorted_png_files(&frames_dir);
    if frames.is_empty() {
        return Err("FFmpeg 未能从视频中解码出任何画面帧".into());
    }

    let mut expected_text: Option<String> = None;
    let mut valid_frames = 0_usize;
    let mut modified_frames = 0_usize;
    let mut corrected_codewords = 0_usize;

    for (index, frame) in frames.iter().enumerate() {
        match extract_image_file(frame, key_source) {
            Ok(report) => {
                if let Some(text) = &expected_text {
                    if text != &report.text {
                        warn!(frame = index + 1, "视频帧提取出的水印文本不一致");
                        continue;
                    }
                } else {
                    expected_text = Some(report.text.clone());
                }
                valid_frames += 1;
                corrected_codewords += report.corrected_codewords;
                if report.integrity != IntegrityStatus::Intact {
                    modified_frames += 1;
                }
            }
            Err(error) => {
                warn!(frame = index + 1, %error, "视频帧水印提取失败");
            }
        }
        if (index + 1) % 25 == 0 || index + 1 == frames.len() {
            info!(processed = index + 1, total = frames.len(), "视频逐帧检测进度");
        }
    }

    // 按需求，只有所有帧均能解码且文本一致时才将视频判定为有效。
    // Per the requirement, a video is valid only when every frame decodes successfully with identical text.
    if valid_frames != frames.len() {
        return Err(format!(
            "视频逐帧校验失败：共 {} 帧，仅 {} 帧成功提取一致水印",
            frames.len(), valid_frames
        ));
    }

    let integrity = if modified_frames == 0 {
        IntegrityStatus::Intact
    } else if modified_frames * 10 < frames.len() {
        IntegrityStatus::Uncertain
    } else {
        IntegrityStatus::Modified
    };

    Ok(VideoDecodeReport {
        text: expected_text.unwrap_or_default(),
        integrity,
        corrected_codewords,
        frame_count: frames.len(),
        valid_frames,
        modified_frames,
    })
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

fn run_command<I>(program: &Path, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let output = hidden_command(program)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("无法启动 {}：{error}", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "FFmpeg 命令执行失败（退出码 {:?}）：{}",
        output.status.code(),
        stderr.trim()
    ))
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
