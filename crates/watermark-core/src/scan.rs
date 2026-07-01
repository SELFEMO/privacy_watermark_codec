use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{
    error::{CoreError, Result},
    watermark::probe_embedded_header_file,
};

const MAX_METADATA_SNIPPETS: usize = 8;
const SNIPPET_RADIUS: usize = 220;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyScanStatus {
    Detected,
    NotDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyScanDetection {
    pub detector: String,
    pub label: String,
    pub content: String,
    pub confidence: String,
    pub needs_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyScanReport {
    pub status: PrivacyScanStatus,
    pub summary: String,
    pub detections: Vec<PrivacyScanDetection>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct Marker {
    keyword: &'static str,
    detector: &'static str,
    label: &'static str,
    confidence: &'static str,
}

const MARKERS: &[Marker] = &[
    Marker { keyword: "c2pa", detector: "metadata", label: "C2PA / Content Credentials", confidence: "high" },
    Marker { keyword: "content credentials", detector: "metadata", label: "C2PA / Content Credentials", confidence: "high" },
    Marker { keyword: "com.adobe.cai", detector: "metadata", label: "Adobe Content Authenticity", confidence: "high" },
    Marker { keyword: "synthid", detector: "metadata", label: "Google SynthID marker", confidence: "medium" },
    Marker { keyword: "stable diffusion", detector: "metadata", label: "Stable Diffusion generation metadata", confidence: "medium" },
    Marker { keyword: "automatic1111", detector: "metadata", label: "Stable Diffusion WebUI metadata", confidence: "medium" },
    Marker { keyword: "comfyui", detector: "metadata", label: "ComfyUI generation workflow", confidence: "medium" },
    Marker { keyword: "invokeai", detector: "metadata", label: "InvokeAI generation metadata", confidence: "medium" },
    Marker { keyword: "midjourney", detector: "metadata", label: "Midjourney metadata", confidence: "medium" },
    Marker { keyword: "dall-e", detector: "metadata", label: "DALL-E / OpenAI metadata", confidence: "medium" },
    Marker { keyword: "openai", detector: "metadata", label: "OpenAI metadata", confidence: "medium" },
    Marker { keyword: "ai-generated", detector: "metadata", label: "AI generation declaration", confidence: "low" },
    Marker { keyword: "ai generated", detector: "metadata", label: "AI generation declaration", confidence: "low" },
    Marker { keyword: "人工智能生成", detector: "metadata", label: "AI generation declaration", confidence: "low" },
    Marker { keyword: "AI生成", detector: "metadata", label: "AI generation declaration", confidence: "low" },
];

pub fn scan_image_file(input_path: impl AsRef<Path>) -> Result<PrivacyScanReport> {
    let input_path = input_path.as_ref();
    let bytes = fs::read(input_path)?;
    let image = image::open(input_path).map_err(|source| CoreError::ImageOpen {
        path: input_path.to_path_buf(),
        source,
    })?;
    let mut detections = Vec::new();

    if let Some(header) = probe_embedded_header_file(input_path)? {
        // 这里仅公开水印头中的非敏感字段，因为本项目的正文载荷经过 ChaCha20-Poly1305 加密，不能也不应绕过密钥直接读取。
        // Only non-sensitive header fields are exposed here because this project's body payload is encrypted with ChaCha20-Poly1305 and must not be bypassed without the key.
        detections.push(PrivacyScanDetection {
            detector: "pww_dct_header".into(),
            label: "本软件加密频域隐私水印".into(),
            content: format!(
                "检测到有效 PWW1 水印头；salt={}，payload={} bytes，route_step={}，strength≈{:.1}。具体水印文本已加密，需要 .key 文件或自定义密码解码。",
                header.salt_hex,
                header.body_len,
                header.route_step,
                header.strength
            ),
            confidence: "high".into(),
            needs_key: true,
        });
    }

    detections.extend(scan_metadata_markers(&bytes));

    if detections.is_empty() {
        return Ok(PrivacyScanReport {
            status: PrivacyScanStatus::NotDetected,
            summary: "未检测出隐私水印，但不确保图片一定不存在隐私水印；部分 AI 水印、专有隐写算法或强加密载荷需要原厂模型、密钥或更高成本的取证流程才能验证。".into(),
            detections,
            width: Some(image.width()),
            height: Some(image.height()),
        });
    }

    Ok(PrivacyScanReport {
        status: PrivacyScanStatus::Detected,
        summary: build_detected_summary(&detections),
        detections,
        width: Some(image.width()),
        height: Some(image.height()),
    })
}

fn scan_metadata_markers(bytes: &[u8]) -> Vec<PrivacyScanDetection> {
    let mut detections = Vec::new();
    let lower = ascii_lowercase(bytes);

    for marker in MARKERS {
        let keyword = marker.keyword.as_bytes();
        let search_space = if keyword.iter().all(u8::is_ascii) {
            lower.as_slice()
        } else {
            bytes
        };

        if let Some(index) = find_subslice(search_space, keyword) {
            // 元数据水印通常以 XMP/PNG 文本块/EXIF 字符串出现，截取上下文比只报告命中词更利于用户判断来源。
            // Metadata watermarks are commonly stored as XMP, PNG text chunks, or EXIF strings, so returning context is more useful than returning only the matched keyword.
            detections.push(PrivacyScanDetection {
                detector: marker.detector.into(),
                label: marker.label.into(),
                content: sanitize_snippet(bytes, index, marker.keyword.len()),
                confidence: marker.confidence.into(),
                needs_key: false,
            });
        }
        if detections.len() >= MAX_METADATA_SNIPPETS {
            break;
        }
    }

    if detections.len() < MAX_METADATA_SNIPPETS {
        detections.extend(scan_stable_diffusion_parameter_block(bytes, detections.len()));
    }

    detections
}

fn scan_stable_diffusion_parameter_block(
    bytes: &[u8],
    existing_count: usize,
) -> Vec<PrivacyScanDetection> {
    let lower = ascii_lowercase(bytes);
    let Some(parameters_index) = find_subslice(&lower, b"parameters") else {
        return Vec::new();
    };
    let nearby_start = parameters_index.saturating_sub(SNIPPET_RADIUS);
    let nearby_end = (parameters_index + SNIPPET_RADIUS * 2).min(lower.len());
    let nearby = &lower[nearby_start..nearby_end];
    let looks_like_generation_params = nearby.windows(b"negative prompt".len()).any(|w| w == b"negative prompt")
        || nearby.windows(b"sampler".len()).any(|w| w == b"sampler")
        || nearby.windows(b"cfg scale".len()).any(|w| w == b"cfg scale")
        || nearby.windows(b"steps:".len()).any(|w| w == b"steps:");

    if !looks_like_generation_params || existing_count >= MAX_METADATA_SNIPPETS {
        return Vec::new();
    }

    vec![PrivacyScanDetection {
        detector: "metadata".into(),
        label: "Stable Diffusion 参数块".into(),
        content: sanitize_snippet(bytes, parameters_index, b"parameters".len()),
        confidence: "medium".into(),
        needs_key: false,
    }]
}

fn build_detected_summary(detections: &[PrivacyScanDetection]) -> String {
    let readable = detections
        .iter()
        .filter(|item| !item.needs_key)
        .count();
    let encrypted = detections.len().saturating_sub(readable);
    match (readable, encrypted) {
        (0, _) => "检测到隐私水印痕迹，但具体内容处于加密状态；请使用解码功能并提供对应 .key 文件或自定义密码读取文本。".into(),
        (_, 0) => "检测到可读的隐私水印/AI 生成元数据，具体内容见下方检测项。".into(),
        _ => "同时检测到可读元数据与加密隐私水印；可读内容见下方，加密水印需提供密钥后才能读取正文。".into(),
    }
}

fn ascii_lowercase(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|byte| byte.to_ascii_lowercase()).collect()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn sanitize_snippet(bytes: &[u8], index: usize, keyword_len: usize) -> String {
    let start = index.saturating_sub(SNIPPET_RADIUS);
    let end = (index + keyword_len + SNIPPET_RADIUS).min(bytes.len());
    let raw = String::from_utf8_lossy(&bytes[start..end]);
    let mut snippet = String::with_capacity(raw.len());

    for ch in raw.chars() {
        if ch.is_control() {
            if !snippet.ends_with(' ') {
                snippet.push(' ');
            }
        } else {
            snippet.push(ch);
        }
    }

    let compact = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = compact.trim_matches(|ch: char| !ch.is_alphanumeric() && !ch.is_ascii_punctuation() && !ch.is_whitespace());
    debug!(chars = trimmed.chars().count(), "提取到疑似水印元数据上下文");
    trimmed.chars().take(520).collect()
}
