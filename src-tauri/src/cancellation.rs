use std::{collections::HashSet, sync::{Arc, Mutex}};

#[derive(Debug, Clone, Default)]
pub struct CancellationRegistry {
    requested: Arc<Mutex<HashSet<String>>>,
}

impl CancellationRegistry {
    pub fn clear(&self, task_id: Option<&str>) {
        if let Some(task_id) = task_id.filter(|value| !value.is_empty()) {
            if let Ok(mut requested) = self.requested.lock() {
                requested.remove(task_id);
            }
        }
    }

    pub fn request_cancel(&self, task_id: &str) {
        if task_id.trim().is_empty() {
            return;
        }
        if let Ok(mut requested) = self.requested.lock() {
            requested.insert(task_id.to_owned());
        }
    }

    pub fn is_cancelled(&self, task_id: Option<&str>) -> bool {
        let Some(task_id) = task_id.filter(|value| !value.is_empty()) else {
            return false;
        };
        self.requested
            .lock()
            .map(|requested| requested.contains(task_id))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct CancellationToken {
    task_id: Option<String>,
    registry: CancellationRegistry,
}

impl CancellationToken {
    pub fn new(task_id: Option<String>, registry: CancellationRegistry) -> Self {
        registry.clear(task_id.as_deref());
        Self { task_id, registry }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.task_id.as_deref()
    }

    pub fn is_cancelled(&self) -> bool {
        self.registry.is_cancelled(self.task_id())
    }

    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            return Err("任务已取消".into());
        }
        Ok(())
    }

}

impl Drop for CancellationToken {
    fn drop(&mut self) {
        // 任务结束后清理取消标记，避免复用同一任务编号时被旧状态误伤。
        // The cancel flag is cleared after the task ends so a reused task id is not affected by stale state.
        let task_id = self.task_id.clone();
        self.registry.clear(task_id.as_deref());
    }
}
