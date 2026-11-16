//! Thin command surface (spec §9.2); business logic lives behind typed APIs.

pub mod auth;
pub mod commits;
pub mod diff;
pub mod distribution;
pub mod history;
pub mod promotion;
pub mod registry;
pub mod system;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::FixtureWorld;
use crate::AppState;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub(crate) fn live_token() -> AppResult<String> {
    crate::github::gh_cli::auth_token().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
    })
}

pub(crate) fn read_world<T>(
    state: &AppState,
    f: impl FnOnce(&FixtureWorld) -> AppResult<T>,
) -> AppResult<T> {
    let world = state.world.lock().map_err(|_| {
        AppError::new(ErrorCode::Internal, "The fixture world lock is poisoned.")
            .with_remedy("Restart the application; report this if it recurs.")
    })?;
    f(&world)
}

pub(crate) fn write_world<T>(
    state: &AppState,
    f: impl FnOnce(&mut FixtureWorld) -> AppResult<T>,
) -> AppResult<T> {
    let mut world = state.world.lock().map_err(|_| {
        AppError::new(ErrorCode::Internal, "The fixture world lock is poisoned.")
            .with_remedy("Restart the application; report this if it recurs.")
    })?;
    f(&mut world)
}

/// Cache-first live read with in-flight coalescing: concurrent commands that
/// compute the same `key` share one `compute` call (one clone+`gh` round),
/// and every result lands in the 90s response cache for later tab visits.
pub(crate) fn cached_json<T>(
    state: &AppState,
    key: &str,
    compute: impl FnOnce() -> AppResult<T>,
) -> AppResult<T>
where
    T: Serialize + DeserializeOwned,
{
    if let Some(hit) = state.cache.get(key) {
        if let Ok(value) = serde_json::from_str::<T>(&hit) {
            return Ok(value);
        }
    }
    let slot = state.singleflight.lock_for(key);
    let _guard = slot.lock().unwrap_or_else(|err| err.into_inner());
    if let Some(hit) = state.cache.get(key) {
        if let Ok(value) = serde_json::from_str::<T>(&hit) {
            state.singleflight.remove_if_unused(key);
            return Ok(value);
        }
    }
    let outcome = compute();
    match outcome {
        // Store before releasing: a newcomer either waits on the still-present
        // slot and double-check-hits, or arrives after removal to a warm cache.
        Ok(value) => {
            if let Ok(encoded) = serde_json::to_string(&value) {
                state.cache.put(key, encoded);
            }
            state.singleflight.remove_if_unused(key);
            Ok(value)
        }
        Err(err) => {
            state.singleflight.remove_if_unused(key);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn cache_hit_skips_compute() {
        let state = AppState::new();
        let calls = AtomicU64::new(0);
        let first: String = cached_json(&state, "k::one", || {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok("value".to_string())
        })
        .expect("compute");
        assert_eq!(first, "value");
        let second: String = cached_json(&state, "k::one", || {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok("other".to_string())
        })
        .expect("cached");
        assert_eq!(second, "value", "second call hits the cache");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_identical_calls_share_one_compute() {
        let state = std::sync::Arc::new(AppState::new());
        let calls = std::sync::Arc::new(AtomicU64::new(0));
        let mut handles = Vec::new();
        // A barrier makes the overlap deterministic. The sleep alone is not
        // enough: a thread scheduled late can miss the slot and recompute,
        // which is why this assertion used to fail intermittently.
        let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
        for _ in 0..8 {
            let state = state.clone();
            let calls = calls.clone();
            let gate = gate.clone();
            handles.push(std::thread::spawn(move || {
                gate.wait();
                std::thread::sleep(std::time::Duration::from_millis(50));
                cached_json(&state, "k::shared", || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    Ok("shared".to_string())
                })
            }));
        }
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.join().expect("thread").expect("value"));
        }
        assert!(results.iter().all(|value| value == "shared"));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "eight concurrent identical requests share one compute"
        );
        assert_eq!(
            state.singleflight.len_for_tests(),
            0,
            "slots are released after use"
        );
    }

    #[test]
    fn errors_are_not_cached() {
        let state = AppState::new();
        let calls = AtomicU64::new(0);
        for _ in 0..2 {
            let failed: AppResult<String> = cached_json(&state, "k::err", || {
                calls.fetch_add(1, Ordering::Relaxed);
                Err(AppError::invalid_request("nope"))
            });
            assert!(failed.is_err());
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn failed_compute_releases_slot_and_caches_nothing() {
        let state = AppState::new();
        let failed: AppResult<String> = cached_json(&state, "k::fails", || {
            Err(AppError::invalid_request("nope"))
        });
        assert!(failed.is_err());
        assert_eq!(
            state.singleflight.len_for_tests(),
            0,
            "a failed compute releases its slot instead of leaking it"
        );
        let ok: String =
            cached_json(&state, "k::fails", || Ok("fresh".to_string())).expect("compute");
        assert_eq!(ok, "fresh", "nothing from the failure was cached");
        assert_eq!(
            state.singleflight.len_for_tests(),
            0,
            "slots are released after use"
        );
    }
}
