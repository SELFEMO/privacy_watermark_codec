use serde::{Deserialize, Serialize};
use watermark_core::{IntegrityStatus, KeyMode, PrivacyScanDetection, PrivacyScanStatus, SyncRegistration, TamperRegion};

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
    pub frame_parallelism: Option<usize>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeItemResult {
    pub input_path: String,
    pub output_path: String,
    pub key_path: Option<String>,
    pub manifest_path: Option<String>,
    pub media_type: MediaType,
    pub psnr: Option<f64>,
    pub frame_count: Option<usize>,
    pub succeeded: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeResponse {
    pub output_root: String,
    pub items: Vec<EncodeItemResult>,
    pub shared_key_path: Option<String>,
    pub manifest_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeRequest {
    pub input_paths: Vec<String>,
    pub key_file: Option<String>,
    pub key_files: Option<Vec<String>>,
    pub custom_password: Option<String>,
    pub frame_parallelism: Option<usize>,
    pub task_id: Option<String>,
}


#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimelineAnomalyKind {
    Insertion,
    Deletion,
    Replacement,
    Duplicate,
    OutOfOrder,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineAnomalyRange {
    pub kind: TimelineAnomalyKind,
    pub physical_start_frame: Option<usize>,
    pub physical_end_frame: Option<usize>,
    pub expected_start_frame: Option<usize>,
    pub expected_end_frame: Option<usize>,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeItemResult {
    pub input_path: String,
    pub succeeded: bool,
    pub error: Option<String>,
    pub media_type: MediaType,
    pub text: String,
    pub integrity: IntegrityStatus,
    pub fingerprint_distance: Option<u32>,
    pub corrected_codewords: usize,
    pub frame_count: Option<usize>,
    pub expected_frame_count: Option<usize>,
    pub valid_frames: Option<usize>,
    pub modified_frames: Option<usize>,
    pub missing_frames: Option<usize>,
    pub inserted_frames: Option<usize>,
    pub deleted_frames: Option<usize>,
    pub duplicate_frames: Option<usize>,
    pub out_of_order_frames: Option<usize>,
    pub replaced_frames: Option<usize>,
    pub unreadable_frames: Option<usize>,
    pub transformed_frames: Option<usize>,
    pub timeline_anomalies: Vec<TimelineAnomalyRange>,
    pub tamper_regions: Vec<TamperRegion>,
    pub sync_registration: Option<SyncRegistration>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeResponse {
    pub items: Vec<DecodeItemResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeItemEvent {
    pub task_id: Option<String>,
    pub current: usize,
    pub total: usize,
    pub item: DecodeItemResult,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub input_paths: Vec<String>,
    pub task_id: Option<String>,
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
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    pub items: Vec<ScanItemResult>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyEvidenceRequest {
    pub manifest_path: String,
    pub key_file: Option<String>,
    pub custom_password: Option<String>,
}


#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Running,
    Completed,
    CompletedWithErrors,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProgressUnitKind {
    File,
    Frame,
    Geometry,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressEvent {
    pub task_id: Option<String>,
    pub task: TaskProgressKind,
    pub phase: String,
    pub message: String,
    pub current: usize,
    pub total: usize,
    pub percent: f64,
    pub current_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TaskOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_fraction: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_succeeded: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_failed: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_kind: Option<ProgressUnitKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_current: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_label: Option<String>,
}

impl TaskProgressEvent {
    pub fn running(
        task_id: Option<String>,
        task: TaskProgressKind,
        phase: impl Into<String>,
        message: impl Into<String>,
        current: usize,
        total: usize,
        percent: f64,
        current_path: Option<String>,
    ) -> Self {
        Self {
            task_id,
            task,
            phase: phase.into(),
            message: message.into(),
            current,
            total,
            percent,
            current_path,
            outcome: Some(TaskOutcome::Running),
            file_fraction: None,
            files_succeeded: None,
            files_failed: None,
            unit_kind: None,
            unit_current: None,
            unit_total: None,
            unit_label: None,
        }
    }
}


#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelTaskRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskProgressKind {
    Encode,
    Decode,
    Scan,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Video,
}
