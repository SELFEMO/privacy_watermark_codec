use crate::{CoreError, Result, WatermarkKey};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    #[default]
    Standard,
    Deep,
}

#[derive(Clone, Default)]
pub struct RecoveryOptions {
    pub mode: RecoveryMode,
    pub cancellation: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub max_duration: Option<Duration>,
}

struct RequestState {
    mode: RecoveryMode,
    deadline: Instant,
    cancellation: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    keys: Vec<([u8; 16], WatermarkKey)>,
    observations: Vec<((usize, u32, u32, u32, u32, u8), Arc<Vec<f32>>)>,
    expected_salt: Option<[u8; 16]>,
}

thread_local! { static REQUEST: RefCell<Option<RequestState>> = const { RefCell::new(None) }; }

pub(crate) struct RequestScope(Option<RequestState>);
impl RequestScope {
    pub(crate) fn enter(options: RecoveryOptions) -> Self {
        let limit = Duration::from_secs(match options.mode {
            RecoveryMode::Standard => 10,
            RecoveryMode::Deep => 60,
        });
        let duration = options.max_duration.map_or(limit, |value| value.min(limit));
        let state = RequestState {
            mode: options.mode,
            deadline: Instant::now() + duration,
            cancellation: options.cancellation,
            keys: Vec::new(),
            observations: Vec::new(),
            expected_salt: None,
        };
        // 截止时间和派生密钥属于整次请求，镜像或候选重试不能重置预算，也不能把密码缓存泄漏到下个任务。
        // Deadlines and derived keys belong to the entire request, so mirror/candidate retries cannot reset the budget or leak password caches into another task.
        Self(REQUEST.with(|slot| slot.replace(Some(state))))
    }
}
impl Drop for RequestScope {
    fn drop(&mut self) {
        REQUEST.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

pub(crate) fn checkpoint() -> Result<()> {
    REQUEST.with(|slot| {
        let state = slot.borrow();
        if let Some(state) = state.as_ref() {
            if state.cancellation.as_ref().is_some_and(|cancel| cancel()) {
                return Err(CoreError::Cancelled);
            }
            if Instant::now() >= state.deadline {
                return Err(CoreError::RecoveryTimeout);
            }
        }
        Ok(())
    })
}
pub(crate) fn stopped() -> bool {
    checkpoint().is_err()
}

pub(crate) fn is_deep() -> bool {
    REQUEST.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|state| matches!(state.mode, RecoveryMode::Deep))
    })
}

pub(crate) fn set_key_source(source: &crate::KeySource) -> Result<()> {
    let salt = match source {
        crate::KeySource::KeyFile(file) => Some(file.to_watermark_key()?.salt),
        crate::KeySource::KeyFiles(files) if files.len() == 1 => {
            Some(files[0].to_watermark_key()?.salt)
        }
        _ => None,
    };
    REQUEST.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.expected_salt = salt;
        }
    });
    Ok(())
}

pub(crate) fn expected_salt() -> Option<[u8; 16]> {
    REQUEST.with(|slot| slot.borrow().as_ref().and_then(|state| state.expected_salt))
}

pub(crate) fn clear_observations() {
    REQUEST.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.observations.clear();
        }
    });
}

pub(crate) fn observations(
    image: &image::RgbaImage,
    px: u32,
    py: u32,
    profile: u8,
) -> Result<Arc<Vec<f32>>> {
    let profile = if profile > 0 { 1 } else { 0 };
    let cache_key = (
        image.as_raw().as_ptr() as usize,
        image.width(),
        image.height(),
        px,
        py,
        profile,
    );
    if let Some(values) = REQUEST.with(|slot| {
        slot.borrow().as_ref().and_then(|state| {
            state
                .observations
                .iter()
                .find(|(key, _)| *key == cache_key)
                .map(|(_, values)| values.clone())
        })
    }) {
        return Ok(values);
    }
    let width = image.width().saturating_sub(px) / 8;
    let height = image.height().saturating_sub(py) / 8;
    let mut values = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        if y % 8 == 0 {
            checkpoint()?;
        }
        for x in 0..width {
            values.push(crate::dwt_qim::read_difference_at_pixel(
                image,
                px + x * 8,
                py + y * 8,
                profile,
            ));
        }
    }
    let values = Arc::new(values);
    // 仅保留当前校正图的四组观测，复用强度和原点重试，同时限制大图缓存占用。
    // Keep four observation grids for the current correction, reusing strength/origin retries while bounding large-image cache memory.
    REQUEST.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            if state.observations.len() >= 4 {
                state.observations.remove(0);
            }
            state.observations.push((cache_key, values.clone()));
        }
    });
    Ok(values)
}

pub(crate) fn cached_key(salt: &[u8; 16]) -> Option<WatermarkKey> {
    REQUEST.with(|slot| {
        slot.borrow().as_ref().and_then(|state| {
            state
                .keys
                .iter()
                .find(|(value, _)| value == salt)
                .map(|(_, key)| key.clone())
        })
    })
}
pub(crate) fn cache_key(salt: [u8; 16], key: &WatermarkKey) {
    REQUEST.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            // 只缓存经完整头校验的有限盐值，避免损坏候选不断增加内存；WatermarkKey 析构时自动清零。
            // Only a bounded set of salts from validated headers is cached, preventing damaged candidates from growing memory; WatermarkKey zeroizes itself on drop.
            if state.keys.len() < 16 {
                state.keys.push((salt, key.clone()));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cancel_after_four_checks() -> RecoveryOptions {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = AtomicUsize::new(0);
        RecoveryOptions {
            cancellation: Some(Arc::new(move || count.fetch_add(1, Ordering::Relaxed) >= 4)),
            ..Default::default()
        }
    }

    #[test]
    fn bch_fft_and_materialization_stop_inside_their_loops() {
        // 信号在计算开始后触发，防止测试只验证入口检查而遗漏真正耗时的内部循环。
        // Trigger stopping after computation begins so tests cover expensive inner loops instead of only entry checks.
        let bits = crate::bch::encode_bytes(&[7; 256]);
        {
            let _scope = RequestScope::enter(cancel_after_four_checks());
            assert!(crate::bch::decode_bits(&bits, 256).is_none());
            assert!(matches!(checkpoint(), Err(CoreError::Cancelled)));
        }
        let image = image::RgbaImage::from_pixel(512, 512, image::Rgba([120, 130, 140, 255]));
        {
            let _scope = RequestScope::enter(cancel_after_four_checks());
            assert!(matches!(
                crate::spectral::grid_hints(&image),
                Err(CoreError::Cancelled)
            ));
        }
        {
            let _scope = RequestScope::enter(cancel_after_four_checks());
            let transform =
                crate::sync::AffineTransform::registered(512, 512, 2.3, 1.0, 0.0, 0.0).unwrap();
            assert!(matches!(
                crate::sync::materialize_transform_checked(&image, &transform, checkpoint),
                Err(CoreError::Cancelled)
            ));
        }
    }

    #[test]
    fn requested_limits_cannot_extend_standard_or_deep_budgets() {
        for (mode, seconds) in [(RecoveryMode::Standard, 10), (RecoveryMode::Deep, 60)] {
            let _scope = RequestScope::enter(RecoveryOptions {
                mode,
                max_duration: Some(Duration::from_secs(120)),
                ..Default::default()
            });
            let deadline = REQUEST.with(|slot| slot.borrow().as_ref().unwrap().deadline);
            clear_observations();
            let same_deadline = REQUEST.with(|slot| slot.borrow().as_ref().unwrap().deadline);
            assert_eq!(deadline, same_deadline);
            assert!(
                deadline.saturating_duration_since(Instant::now()) <= Duration::from_secs(seconds)
            );
        }
    }
    #[test]
    fn expired_budget_and_early_cancellation_are_distinct() {
        {
            let _scope = RequestScope::enter(RecoveryOptions {
                max_duration: Some(Duration::ZERO),
                ..Default::default()
            });
            assert!(matches!(checkpoint(), Err(CoreError::RecoveryTimeout)));
        }
        {
            let _scope = RequestScope::enter(RecoveryOptions {
                cancellation: Some(Arc::new(|| true)),
                ..Default::default()
            });
            assert!(matches!(checkpoint(), Err(CoreError::Cancelled)));
        }
        assert!(checkpoint().is_ok());
    }
}
