use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

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

#[derive(Debug)]
pub struct CancellationToken {
    task_id: Option<String>,
    registry: CancellationRegistry,
}

impl CancellationToken {
    pub fn new(task_id: Option<String>, registry: CancellationRegistry) -> Self {
        // 取消可能在工作线程获得执行机会前到达，创建令牌时保留标记才能避免启动竞态吞掉用户操作。
        // Cancellation can arrive before a worker is scheduled, so token creation preserves the flag instead of losing the user's request in a startup race.
        Self { task_id, registry }
    }

    pub fn task_id(&self) -> Option<&str> {
        self.task_id.as_deref()
    }

    pub fn is_cancelled(&self) -> bool {
        self.registry.is_cancelled(self.task_id())
    }

    pub fn callback(&self) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let registry = self.registry.clone();
        let task_id = self.task_id.clone();
        // 内层计算通过只读闭包检查取消，无需克隆拥有清理职责的令牌，避免工作线程退出时过早清空标记。
        // Inner computations check cancellation through a read-only closure instead of cloning the cleanup-owning token, preventing workers from clearing flags prematurely.
        Arc::new(move || registry.is_cancelled(task_id.as_deref()))
    }

    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            return Err("任务已取消".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_worker_start_is_preserved_until_completion() {
        let registry = CancellationRegistry::default();
        registry.request_cancel("queued-task");
        let token = CancellationToken::new(Some("queued-task".into()), registry.clone());
        assert!(token.check().is_err());
        drop(token);
        assert!(!registry.is_cancelled(Some("queued-task")));
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
