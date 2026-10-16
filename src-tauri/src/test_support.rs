//! Crate-wide lock serializing tests that mutate process-global env vars.
//!
//! [`std::env::set_var`] / [`std::env::remove_var`] (and reads like
//! [`std::env::var`]) act on state shared by the whole test process. Cargo
//! runs `#[test]`s on threads in one binary, so two tests touching env at
//! once race: e.g. a `live_read` test resolving `VOS_TARGET_REPO_DIR` can run
//! while `checkouts` has `VOS_CHECKOUTS_DIR` set, or while `gh_cli` has
//! scrubbed `GH_TOKEN`, sending each down a different code path and failing
//! rarely depending on thread order.
//!
//! Per-module locks do NOT fix this: each module would still mutate the same
//! global env concurrently with the others. Every env-mutating test in this
//! crate must therefore hold THIS single lock for its whole duration.

use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_guard() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

/// Hold the returned guard for the whole env-mutating test; dropping it
/// releases the lock. Tolerates poisoning so one panicking test cannot wedge
/// the rest of the suite.
pub fn env_lock() -> MutexGuard<'static, ()> {
    env_guard().lock().unwrap_or_else(|err| err.into_inner())
}
