use serde::{Deserialize, Serialize};
use watermark_core::{IntegrityStatus, KeyMode, PrivacyScanDetection, PrivacyScanStatus};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeRequest {
    pub input_paths: Vec<String>,
    pub output_dir: String,
    pub text: String,
    pub key_mode: KeyMode,
    pub custom_password: Option<String>,
    pub write_key_file: bool,
    pub strength: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeItemResult {
    pub input_path: String,
    pub output_path: String,
    pub key_path: Option<String>,
    pub media_type: MediaType,
    pub psnr: Option<f64>,
    pub frame_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeResponse {
    pub output_root: String,
    pub items: Vec<EncodeItemResult>,
    pub shared_key_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeRequest {
    pub input_paths: Vec<String>,
    pub key_file: Option<String>,
    pub custom_password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeItemResult {
    pub input_path: String,
    pub media_type: MediaType,
    pub text: String,
    pub integrity: IntegrityStatus,
    pub fingerprint_distance: Option<u32>,
    pub corrected_codewords: usize,
    pub frame_count: Option<usize>,
    pub valid_frames: Option<usize>,
    pub modified_frames: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeResponse {
    pub items: Vec<DecodeItemResult>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub input_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanItemResult {
    pub input_path: String,
    pub status: PrivacyScanStatus,
    pub summary: String,
    pub detections: Vec<PrivacyScanDetection>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    pub items: Vec<ScanItemResult>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Video,
}
