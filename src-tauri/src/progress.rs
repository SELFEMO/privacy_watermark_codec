use crate::models::{
    DecodeItemEvent, ProgressUnitKind, TaskOutcome, TaskProgressEvent, TaskProgressKind,
};
use tauri::{AppHandle, Emitter};

pub const TASK_PROGRESS_EVENT: &str = "pwc-task-progress";
pub const DECODE_ITEM_EVENT: &str = "pwc-decode-item";

pub fn emit_task_progress(app: &AppHandle, event: TaskProgressEvent) {
    // 进度事件只携带任务阶段和路径摘要，不包含密钥或水印正文，避免界面反馈泄露敏感内容。
    // Progress events carry only task stage and path summaries, not keys or watermark text, to avoid leaking sensitive data through UI feedback.
    let _ = app.emit(TASK_PROGRESS_EVENT, event);
}


pub fn emit_decode_item(app: &AppHandle, event: DecodeItemEvent) {
    // 每个媒体完成后立即发送独立结果，是为了让长批次在后台继续时仍可逐项查看结论，而最终响应继续作为完整快照兜底。
    // Emitting each media result immediately lets users inspect completed items while a long batch continues, while the final response remains the authoritative full snapshot.
    let _ = app.emit(DECODE_ITEM_EVENT, event);
}

pub fn clamp_percent(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

pub fn batch_percent(file_index: usize, file_total: usize, file_fraction: f64) -> f64 {
    let total = file_total.max(1) as f64;
    // 批次条只按文件序号叠加当前文件内完成比例，避免把几何候选数或视频帧数当成文件计数。
    // The batch bar uses file index plus the current file's fraction so geometry candidates or video frames never masquerade as file counts.
    clamp_percent(((file_index as f64 + file_fraction.clamp(0.0, 1.0)) / total) * 100.0)
}

pub struct ProgressDetail {
    pub current: usize,
    pub total: usize,
    pub percent: f64,
    pub current_path: Option<String>,
    pub outcome: TaskOutcome,
    pub file_fraction: Option<f64>,
    pub files_succeeded: Option<usize>,
    pub files_failed: Option<usize>,
    pub unit_kind: Option<ProgressUnitKind>,
    pub unit_current: Option<usize>,
    pub unit_total: Option<usize>,
    pub unit_label: Option<String>,
}

pub fn progress_event_detailed(
    task_id: Option<String>,
    task: TaskProgressKind,
    phase: &str,
    message: impl Into<String>,
    detail: ProgressDetail,
) -> TaskProgressEvent {
    TaskProgressEvent {
        task_id,
        task,
        phase: phase.to_owned(),
        message: message.into(),
        current: detail.current,
        total: detail.total,
        percent: clamp_percent(detail.percent),
        current_path: detail.current_path,
        outcome: Some(detail.outcome),
        file_fraction: detail.file_fraction,
        files_succeeded: detail.files_succeeded,
        files_failed: detail.files_failed,
        unit_kind: detail.unit_kind,
        unit_current: detail.unit_current,
        unit_total: detail.unit_total,
        unit_label: detail.unit_label,
    }
}

pub fn emit_progress_detailed(
    app: &AppHandle,
    task_id: Option<String>,
    task: TaskProgressKind,
    phase: &str,
    message: impl Into<String>,
    detail: ProgressDetail,
) {
    emit_task_progress(
        app,
        progress_event_detailed(task_id, task, phase, message, detail),
    );
}

pub fn decode_geometry_file_fraction(
    stage: &str,
    current: usize,
    total: usize,
    sub_progress: f32,
) -> f64 {
    if stage == "verify" {
        let total = total.max(1) as f64;
        let done = (current.saturating_sub(1) as f64 + f64::from(sub_progress.clamp(0.0, 1.0)))
            .clamp(0.0, total);
        // 评分最多走到 90%，完整验证占用 90%–98%，才能把 1025/1025 和“还在插值/扫头”区分开。
        // Scoring stops at 90% and full verification uses 90%–98%, so 1025/1025 is no longer indistinguishable from a silent warp/header scan.
        0.90 + (done / total) * 0.08
    } else {
        let fraction = current as f64 / total.max(1) as f64;
        0.50 + fraction * 0.40
    }
}

#[cfg(test)]
mod tests {
    use super::{
        batch_percent, decode_geometry_file_fraction, progress_event_detailed, ProgressDetail,
    };
    use crate::models::{ProgressUnitKind, TaskOutcome, TaskProgressKind};

    #[test]
    fn batch_percent_uses_file_index_plus_fraction() {
        let percent = batch_percent(2, 8, 0.5);
        assert!((percent - 31.25).abs() < 1e-9);
    }

    #[test]
    fn geometry_fraction_does_not_use_candidate_as_file_counter() {
        let file_fraction = decode_geometry_file_fraction("rotation-layout-near", 512, 1025, 0.0);
        assert!(file_fraction > 0.50 && file_fraction < 0.90);
        // 调用方必须把 512/1025 放进 unit*，而不是 current/total
    }

    #[test]
    fn task_outcome_serializes_completed_with_errors_as_snake_case() {
        let value = serde_json::to_value(TaskOutcome::CompletedWithErrors).unwrap();
        assert_eq!(value, "completed_with_errors");
    }

    #[test]
    fn geometry_progress_keeps_file_counters_separate_from_unit_fields() {
        let file_fraction = decode_geometry_file_fraction("rotation-layout-near", 512, 1025, 0.0);
        let event = progress_event_detailed(
            None,
            TaskProgressKind::Decode,
            "registering-image",
            "近角度",
            ProgressDetail {
                current: 3,
                total: 8,
                percent: batch_percent(2, 8, file_fraction),
                current_path: None,
                outcome: TaskOutcome::Running,
                file_fraction: Some(file_fraction),
                files_succeeded: Some(2),
                files_failed: Some(0),
                unit_kind: Some(ProgressUnitKind::Geometry),
                unit_current: Some(512),
                unit_total: Some(1025),
                unit_label: Some("近角度布局搜索 +2.00° ×0.956".into()),
            },
        );
        assert_eq!(event.current, 3);
        assert_eq!(event.total, 8);
        assert_eq!(event.unit_current, Some(512));
        assert_eq!(event.unit_total, Some(1025));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["current"], 3);
        assert_eq!(json["total"], 8);
        assert_eq!(json["unitKind"], "geometry");
        assert_eq!(json["unitCurrent"], 512);
        assert_eq!(json["outcome"], "running");
        assert_ne!(json["current"], json["unitCurrent"]);
    }

    #[test]
    fn scoring_cap_stops_at_ninety_percent_of_the_file() {
        let fraction = decode_geometry_file_fraction("rotation-layout-far", 1025, 1025, 0.0);
        assert!((fraction - 0.90).abs() < 1e-9);
    }

    #[test]
    fn verify_progress_fills_the_reserved_tail() {
        let start = decode_geometry_file_fraction("verify", 1, 4, 0.0);
        let end = decode_geometry_file_fraction("verify", 4, 4, 1.0);
        assert!((start - 0.90).abs() < 1e-9);
        assert!((end - 0.98).abs() < 1e-9);
    }

    #[test]
    fn encode_completed_with_errors_reports_nonzero_success_and_failure_counts() {
        // 编码收尾必须带上成功/失败计数，否则界面会把 None 显示成 0/0，和“部分文件失败”互相矛盾。
        // Encode completion must carry success/failure counts, otherwise the UI renders None as 0/0 against a partial-failure caption.
        let event = progress_event_detailed(
            None,
            TaskProgressKind::Encode,
            "completed_with_errors",
            "批量编码任务结束，部分文件失败",
            ProgressDetail {
                current: 2,
                total: 2,
                percent: 100.0,
                current_path: None,
                outcome: TaskOutcome::CompletedWithErrors,
                file_fraction: Some(1.0),
                files_succeeded: Some(1),
                files_failed: Some(1),
                unit_kind: Some(ProgressUnitKind::File),
                unit_current: None,
                unit_total: None,
                unit_label: None,
            },
        );
        assert_eq!(event.outcome, Some(TaskOutcome::CompletedWithErrors));
        assert_eq!(event.files_succeeded, Some(1));
        assert_eq!(event.files_failed, Some(1));
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["filesSucceeded"], 1);
        assert_eq!(json["filesFailed"], 1);
        assert_ne!(json["filesSucceeded"], 0);
        assert_ne!(json["filesFailed"], 0);
    }
}
