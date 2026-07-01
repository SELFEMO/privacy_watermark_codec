use crate::models::TaskProgressEvent;
use tauri::{AppHandle, Emitter};

pub const TASK_PROGRESS_EVENT: &str = "pwc-task-progress";

pub fn emit_task_progress(app: &AppHandle, event: TaskProgressEvent) {
    // 进度事件只携带任务阶段和路径摘要，不包含密钥或水印正文，避免界面反馈泄露敏感内容。
    // Progress events carry only task stage and path summaries, not keys or watermark text, to avoid leaking sensitive data through UI feedback.
    let _ = app.emit(TASK_PROGRESS_EVENT, event);
}

pub fn clamp_percent(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}
