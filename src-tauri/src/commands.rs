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
    media::{detect_media_type, safe_stem},
    models::{
        DecodeItemResult, DecodeRequest, DecodeResponse, EncodeItemResult, EncodeRequest,
        EncodeResponse, MediaType, ScanItemResult, ScanRequest, ScanResponse,
    },
    video::{decode_video, encode_video},
};

#[tauri::command]
pub async fn encode_media(
    app: tauri::AppHandle,
    request: EncodeRequest,
) -> Result<EncodeResponse, String> {
    tauri::async_runtime::spawn_blocking(move || encode_media_blocking(app, request))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

#[tauri::command]
pub async fn decode_media(
    app: tauri::AppHandle,
    request: DecodeRequest,
) -> Result<DecodeResponse, String> {
    tauri::async_runtime::spawn_blocking(move || decode_media_blocking(app, request))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

#[tauri::command]
pub async fn scan_privacy_watermark(request: ScanRequest) -> Result<ScanResponse, String> {
    tauri::async_runtime::spawn_blocking(move || scan_privacy_watermark_blocking(request))
        .await
        .map_err(|error| format!("后台任务异常终止：{error}"))?
}

fn encode_media_blocking(app: tauri::AppHandle, request: EncodeRequest) -> Result<EncodeResponse, String> {
    validate_encode_request(&request)?;
    let output_base = PathBuf::from(&request.output_dir);
    fs::create_dir_all(&output_base).map_err(|error| error.to_string())?;

    info!(
        files = request.input_paths.len(),
        ?request.key_mode,
        output = %output_base.display(),
        "收到批量编码任务"
    );

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

    if let Some(key) = &shared_key {
        let should_write = request.key_mode != KeyMode::Custom || request.write_key_file;
        if should_write {
            let file_name = if request.key_mode == KeyMode::Shared {
                "shared.key"
            } else {
                "custom.key"
            };
            let key_path = output_root.join(file_name);
            key.to_key_file()
                .write(&key_path)
                .map_err(|error| error.to_string())?;
            shared_key_path = Some(key_path.display().to_string());
            info!(path = %key_path.display(), "已写入批次密钥文件");
        }
    }

    let mut items = Vec::with_capacity(request.input_paths.len());
    for input_string in &request.input_paths {
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            return Err(format!("输入文件不存在：{}", input.display()));
        }
        let media_type = detect_media_type(&input)
            .ok_or_else(|| format!("不支持的文件类型：{}", input.display()))?;
        let stem = safe_stem(&input);

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

        let result = match media_type {
            MediaType::Image => {
                let report = embed_image_file(
                    &input,
                    &output,
                    &EmbedOptions {
                        text: request.text.clone(),
                        key,
                        strength: request.strength,
                        media_kind: "image".into(),
                    },
                )
                .map_err(|error| error.to_string())?;
                EncodeItemResult {
                    input_path: input.display().to_string(),
                    output_path: output.display().to_string(),
                    key_path: key_path.map(|path| path.display().to_string()),
                    media_type,
                    psnr: Some(report.psnr),
                    frame_count: None,
                }
            }
            MediaType::Video => {
                let report = encode_video(&app, &input, &output, &request.text, &key, request.strength)?;
                EncodeItemResult {
                    input_path: input.display().to_string(),
                    output_path: output.display().to_string(),
                    key_path: key_path.map(|path| path.display().to_string()),
                    media_type,
                    psnr: None,
                    frame_count: Some(report.frame_count),
                }
            }
        };
        items.push(result);
    }

    info!(completed = items.len(), "批量编码任务完成");
    Ok(EncodeResponse {
        output_root: output_root.display().to_string(),
        items,
        shared_key_path,
    })
}


fn scan_privacy_watermark_blocking(request: ScanRequest) -> Result<ScanResponse, String> {
    if request.input_paths.is_empty() {
        return Err("请至少选择一张待检测图片".into());
    }
    info!(files = request.input_paths.len(), "收到未知来源图片隐私水印扫描任务");

    let mut items = Vec::with_capacity(request.input_paths.len());
    for input_string in &request.input_paths {
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            return Err(format!("输入文件不存在：{}", input.display()));
        }
        let media_type = detect_media_type(&input)
            .ok_or_else(|| format!("不支持的文件类型：{}", input.display()))?;
        if media_type != MediaType::Image {
            return Err(format!("未知来源隐私水印扫描当前仅支持图片：{}", input.display()));
        }

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
    }

    info!(completed = items.len(), "未知来源图片隐私水印扫描完成");
    Ok(ScanResponse { items })
}

fn decode_media_blocking(app: tauri::AppHandle, request: DecodeRequest) -> Result<DecodeResponse, String> {
    if request.input_paths.is_empty() {
        return Err("请至少选择一个待解码媒体文件".into());
    }
    let key_source = build_key_source(&request)?;
    info!(files = request.input_paths.len(), "收到批量解码任务");

    let mut items = Vec::with_capacity(request.input_paths.len());
    for input_string in &request.input_paths {
        let input = PathBuf::from(input_string);
        if !input.is_file() {
            return Err(format!("输入文件不存在：{}", input.display()));
        }
        let media_type = detect_media_type(&input)
            .ok_or_else(|| format!("不支持的文件类型：{}", input.display()))?;

        let result = match media_type {
            MediaType::Image => {
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
                }
            }
            MediaType::Video => {
                let report = decode_video(&app, &input, &key_source)?;
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
                }
            }
        };
        items.push(result);
    }

    info!(completed = items.len(), "批量解码任务完成");
    Ok(DecodeResponse { items })
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
