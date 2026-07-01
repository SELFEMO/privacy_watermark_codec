use std::path::Path;

use crate::models::MediaType;

pub fn detect_media_type(path: &Path) -> Option<MediaType> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" => Some(MediaType::Image),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" => Some(MediaType::Video),
        _ => None,
    }
}

pub fn safe_stem(path: &Path) -> String {
    let raw = path.file_stem().and_then(|value| value.to_str()).unwrap_or("media");
    let sanitized: String = raw
        .chars()
        .map(|ch| if ch.is_alphanumeric() || matches!(ch, '-' | '_') { ch } else { '_' })
        .collect();
    if sanitized.is_empty() { "media".into() } else { sanitized }
}
